import assert from 'node:assert/strict';
import { puzzleHtml } from '../../scripts/browser-tools.mjs';

const GAME = 'https://www.neopets.com/medieval/shapeshifter.phtml';
const ACTION = '/medieval/process_shapeshifter.phtml';

// No requests reach Neopets. Model its page links and a server-side referrer check.
export async function testMoveScript(browser, script, puzzle, { hostedUrl, hard } = {}) {
    const scenarios = hostedUrl
        ? ['success', 'changed-before-moves', 'solver-error', 'cancel-search', 'stop']
        : ['success', 'wrong-board', 'refused', 'uncertain-response', 'changed-board', 'stop'];
    for (const scenario of scenarios) {
        // A dead proxy also blocks requests that might escape interception.
        const context = await browser.newContext({ proxy: {
            server: 'http://127.0.0.1:1',
            ...(hostedUrl ? { bypass: new URL(hostedUrl).hostname } : {}),
        } });
        if (hostedUrl?.startsWith('http:') && browser.browserType().name() === 'chromium') {
            await context.grantPermissions(['local-network-access'], { origin: new URL(GAME).origin });
        }
        const page = await context.newPage();
        const logs = [];
        const errors = [];
        const requests = [];
        const state = structuredClone(scenario === 'cancel-search' ? hard : puzzle);
        let moves = 0;
        let inFlight = 0;
        let rejected = 0;
        let reads = 0;
        if (scenario === 'wrong-board') state.board[0][0] = (state.board[0][0] + 1) % state.m;
        page.on('console', entry => logs.push(entry.text()));
        page.on('pageerror', error => errors.push(String(error)));
        await context.addCookies([{ name: 'test_session', value: 'fixture', url: GAME }]);

        function gameHtml() {
            if (!state.pieces.length) return `<b>LEVEL ${state.level}</b><b>You Won!</b>`;
            let html = puzzleHtml(state);
            for (let row = 0; row <= state.rows - state.pieces[0].length; row++) {
                for (let col = 0; col <= state.columns - state.pieces[0][0].length; col++) {
                    html += `<a href="${ACTION}?type=action&amp;posx=${col}&amp;posy=${row}&amp;turn=${moves}">`
                        + `<img name="i${col}_${row}"></a>`;
                }
            }
            return html;
        }

        await context.route('**/*', async route => {
            const request = route.request();
            const url = new URL(request.url());
            if (hostedUrl && url.origin === new URL(hostedUrl).origin) {
                if (scenario === 'solver-error' && url.pathname.endsWith('/search-worker.js')) {
                    await route.fulfill({ contentType: 'text/javascript', body: 'throw new Error("Test solver failed");' });
                    return;
                }
                return route.continue();
            }
            if (url.origin === 'https://solver.example.test') {
                await route.fulfill({ contentType: 'text/html', body:
                    `<a id="old-link" href="https://www.neopets.com${ACTION}?type=action&posx=0&posy=0">Place</a>` });
                return;
            }
            if (url.origin !== new URL(GAME).origin) return route.abort();
            if (url.pathname === '/medieval/shapeshifter.phtml') {
                if (++reads === 3 && scenario === 'changed-before-moves') {
                    state.board[0][0] = (state.board[0][0] + 1) % state.m;
                }
                await route.fulfill({ contentType: 'text/html', body: gameHtml() });
                return;
            }
            if (url.pathname !== ACTION) return route.abort();
            const headers = await request.allHeaders();
            if (headers.referer !== GAME || !headers.cookie?.includes('test_session=fixture')) {
                rejected++;
                await route.fulfill({ contentType: 'text/html', body: 'You came from the wrong place!' });
                return;
            }
            requests.push({ at: Date.now(), headers, url: url.href, overlap: inFlight });
            inFlight++;
            await new Promise(resolve => setTimeout(resolve, 25));
            if (scenario === 'refused') {
                inFlight--;
                await route.fulfill({ contentType: 'text/html', body: 'You came from the wrong place!' });
                return;
            }
            const row = Number(url.searchParams.get('posy'));
            const col = Number(url.searchParams.get('posx'));
            assert.equal(url.searchParams.get('turn'), String(moves), 'Use the latest page link');
            const piece = state.pieces.shift();
            assert(piece, 'No request after the final piece');
            assert(row >= 0 && col >= 0 && row + piece.length <= state.rows && col + piece[0].length <= state.columns);
            piece.forEach((cells, r) => cells.forEach((active, c) => {
                if (active) state.board[row + r][col + c] = (state.board[row + r][col + c] + state.m - 1) % state.m;
            }));
            moves++;
            inFlight--;
            if (scenario === 'uncertain-response') {
                await route.fulfill({ status: 500, body: 'The move happened, but its response failed.' });
            } else {
                if (scenario === 'changed-board') state.board[0][0] = (state.board[0][0] + 1) % state.m;
                // Model the final HTML returned after the action's redirect.
                await route.fulfill({ contentType: 'text/html', body: gameHtml() });
            }
        });
        try {
            if (scenario === 'success' && !hostedUrl) {
                await page.goto('https://solver.example.test');
                await page.evaluate(source => { (0, eval)(source); }, script);
                assert(logs.some(text => text.includes('Neopets Shapeshifter game tab')));
                await Promise.all([page.waitForURL(`**${ACTION}*`), page.locator('#old-link').click()]);
                assert.equal(rejected, 1, 'Direct links from another site fail the fixture check');
                assert.equal(moves, 0);
            }
            await page.goto(GAME);
            const searching = scenario === 'cancel-search' ? page.waitForEvent('console', {
                predicate: entry => entry.text() === 'Searching (2 minute budget)…', timeout: 60000,
            }) : null;
            await page.evaluate(source => {
                (0, eval)(source);
                (0, eval)(source);
            }, script);
            if (searching) {
                await searching;
                await page.evaluate(() => window.shapeshifterMoves.stop());
            }
            async function assertSolved() {
                await page.waitForFunction(total => document.body.textContent.includes('You Won!')
                    || (window.shapeshifterMoves?.running === false && window.shapeshifterMoves.completed < total),
                puzzle.pieces.length, { timeout: 60000 });
                assert((await page.locator('body').innerText()).includes('You Won!'), logs.join('\n'));
                assert.equal(moves, puzzle.pieces.length);
                assert.equal(requests.length, moves);
                assert(state.board.every(row => row.every(value => value === 0)), 'Replay solves the original board');
                for (let i = 1; i < requests.length; i++) {
                    assert(requests[i].at - requests[i - 1].at >= 900, 'Wait between confirmed moves');
                }
            }
            if (scenario === 'success') {
                await assertSolved();
            } else {
                if (scenario === 'stop') {
                    await page.waitForFunction(() => window.shapeshifterMoves?.completed === 1);
                    await page.evaluate(() => window.shapeshifterMoves.stop());
                }
                await page.waitForFunction(() => window.shapeshifterMoves?.running === false);
                assert.equal(requests.length, ['wrong-board', 'changed-before-moves', 'solver-error', 'cancel-search'].includes(scenario) ? 0 : 1,
                    'Stop without retrying or placing more pieces');
                assert(logs.some(text => /stopped/i.test(text)), logs.join('\n'));
                if (scenario === 'refused') assert(logs.some(text => text.includes('wrong place')));
                if (scenario === 'stop') {
                    await page.reload();
                    await page.evaluate(source => { (0, eval)(source); }, script);
                    await assertSolved();
                }
            }
            assert(requests.every(request => request.overlap === 0), 'Requests must be sequential');
            assert(logs.some(text => text.includes('already running')), 'Repeated pastes do not start another runner');
            assert.deepEqual(errors, []);
            assert.equal(await page.locator('iframe').count(), 0, 'Release the hosted solver after completion or cancellation');
            if (hostedUrl && ['success', 'cancel-search'].includes(scenario)) {
                const workers = browser.browserType().name() === 'chromium'
                    ? await page.evaluate(() => navigator.hardwareConcurrency) : 1;
                assert(logs.some(text => text === `Solver ready: ${workers} search workers.`), logs.join('\n'));
                assert.equal(logs.some(text => text.includes('Shared memory is unavailable')), browser.browserType().name() !== 'chromium');
            }
        } catch (error) {
            throw new Error(`Scenario: ${scenario}\n${logs.join('\n')}`, { cause: error });
        } finally {
            await context.close();
        }
    }
    console.log(hostedUrl ? 'PASS console auto-solve: hosted workers, browser isolation, replay, live-state recheck, failure, cancellation, stop/resume'
        : 'PASS console move script: same-origin requests, live links, replay, delays, state checks, stop/resume, no retries');
}
