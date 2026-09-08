import assert from 'node:assert/strict';
import { puzzleHtml } from '../../scripts/browser-tools.mjs';

const GAME = 'https://www.neopets.com/medieval/shapeshifter.phtml';
const ACTION = '/medieval/process_shapeshifter.phtml';
const TIMEOUT_PUZZLE = { level: 10, m: 2, rows: 3, columns: 3,
    board: [[1, 0, 0], [0, 0, 0], [0, 0, 1]], pieces: [[[true]], [[true]]] };
const UNEXPECTED_HTML = '<!doctype html><title>Unexpected game response</title>\n<p>"Try again" &amp; continue.</p>';

// No requests reach Neopets. Model its page links and a server-side referrer check.
export async function testMoveScript(browser, script, puzzle, { hostedUrl, hard } = {}) {
    const scenarios = hostedUrl
        ? ['success', 'start-lost', 'changed-before-moves', 'solver-error', 'timeout-restart', 'timeout-win', 'timeout-stop',
            'timeout-changed', 'timeout-bad-form', 'timeout-restart-error', 'unexpected-page', 'unexpected-final',
            'network-error', 'cancel-search', 'stop']
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
        if (scenario === 'cancel-search') await page.clock.install();
        const logs = [];
        const routineLogs = [];
        const errors = [];
        const requests = [];
        const timeout = scenario.startsWith('timeout-');
        const state = structuredClone(scenario === 'start-lost' ? { ...TIMEOUT_PUZZLE, level: puzzle.level, pieces: [] }
            : timeout || scenario === 'unexpected-final' ? { ...TIMEOUT_PUZZLE, level: puzzle.level }
            : scenario === 'cancel-search' ? hard : puzzle);
        if (scenario === 'timeout-win') state.board = state.board.map(row => row.map(() => 0));
        let moves = 0;
        let restarts = 0;
        let inFlight = 0;
        let rejected = 0;
        let reads = 0;
        if (scenario === 'wrong-board') state.board[0][0] = (state.board[0][0] + 1) % state.m;
        page.on('console', entry => {
            logs.push(entry.text());
            if (['log', 'info'].includes(entry.type())) routineLogs.push(entry.text());
        });
        page.on('pageerror', error => errors.push(String(error)));
        await context.addCookies([{ name: 'test_session', value: 'fixture', url: GAME }]);

        function gameHtml() {
            if (!state.pieces.length) {
                if (state.board.every(row => row.every(value => value === 0))) return `<b>LEVEL ${state.level}</b><b>You Won!</b>`;
                // The saved loss page has separate start_game and redo_game POST forms.
                const action = scenario === 'timeout-bad-form' ? 'https://example.test/restart' : `${ACTION}?type=init`;
                return `<b>LEVEL ${state.level}</b><b>You Lost!</b><form name="start_game" method="post" action="${action}">`
                    + '<input type="hidden" name="turn" value="2"><input type="submit" value="Try again?"></form>'
                    + `<form name="redo_game" method="post" action="${ACTION}?type=init"><select name="past_level"><option value="1">1</option></select></form>`;
            }
            let html = puzzleHtml(scenario === 'timeout-restart' ? { ...state, icons: ['swo', 'hel'] } : state);
            if (moves % 2 === 1) {
                html = '<table><tr><td>Sinsi says: match the symbol above the word <b>GOAL</b> to win.</td></tr></table>' + html;
            }
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
                if (timeout && restarts === 0 && url.pathname.endsWith('/console-solver.js')) {
                    const response = await route.fetch();
                    const body = (await response.text()).replace('client.solve(event.data.puzzle, event.data.budgetMs)', 'client.solve(event.data.puzzle, 0)');
                    await route.fulfill({ response, body });
                    return;
                }
                if (scenario === 'solver-error' && url.pathname.endsWith('/search-worker.js')) {
                    const response = await route.fetch();
                    await route.fulfill({ response, body: 'throw new Error("Test solver failed");' });
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
                if (++reads === 3 && ['changed-before-moves', 'timeout-changed'].includes(scenario)) {
                    state.board[0][0] = (state.board[0][0] + 1) % state.m;
                }
                // The final move can be confirmed before the win-page navigation finishes.
                if (!state.pieces.length) await new Promise(resolve => setTimeout(resolve, 250));
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
            if (url.searchParams.get('type') === 'init') {
                assert((timeout || scenario === 'start-lost') && !state.pieces.length, 'Restart only after placing every piece');
                assert.equal(request.method(), 'POST');
                assert.equal(request.postData(), 'turn=2', 'Submit the live Try Again form, without the level picker');
                assert.equal(restarts++, 0, 'Do not repeat the restart request');
                Object.assign(state, structuredClone(puzzle));
                await route.fulfill(scenario === 'timeout-restart-error'
                    ? { status: 500, body: 'The new game started, but its response failed.' }
                    : { contentType: 'text/html', body: gameHtml() });
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
            if (timeout && restarts === 0) assert.deepEqual([row, col], [0, 0], 'Every forfeited piece goes at the top left');
            assert.equal(url.searchParams.get('turn'), String(moves), 'Use the latest page link');
            const piece = state.pieces.shift();
            assert(piece, 'No request after the final piece');
            assert(row >= 0 && col >= 0 && row + piece.length <= state.rows && col + piece[0].length <= state.columns);
            piece.forEach((cells, r) => cells.forEach((active, c) => {
                if (active) state.board[row + r][col + c] = (state.board[row + r][col + c] + state.m - 1) % state.m;
            }));
            moves++;
            inFlight--;
            if (scenario === 'network-error') {
                await route.abort('failed');
            } else if (scenario === 'unexpected-page' || (scenario === 'unexpected-final' && !state.pieces.length)) {
                await route.fulfill({ contentType: 'text/html', body: UNEXPECTED_HTML });
            } else if (scenario === 'uncertain-response') {
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
            await page.evaluate(source => {
                (0, eval)(source);
                (0, eval)(source);
            }, script);
            const progress = page.locator('#shapeshifter-status .solver-progress');
            const badge = page.locator('#shapeshifter-status .solver-features');
            if (scenario === 'cancel-search') {
                await page.waitForFunction(() => window.shapeshifterMoves?.phase === 'loading');
                assert.equal(await progress.textContent(), 'Loading solver…');
                assert.equal(await page.locator('#shapeshifter-status [role="timer"]').count(), 0, 'Startup does not consume the search countdown');
            }
            if (hostedUrl && ['success', 'cancel-search'].includes(scenario)) {
                await page.waitForFunction(() => !!window.shapeshifterMoves?.solver?.workers, null, { timeout: 60000 });
                const info = await page.evaluate(() => shapeshifterMoves.solver);
                const threaded = browser.browserType().name() === 'chromium';
                assert.equal(info.threaded, threaded);
                assert.equal(info.workers, threaded ? await page.evaluate(() => navigator.hardwareConcurrency) : 1);
            }
            if (scenario === 'cancel-search') {
                await page.waitForFunction(() => window.shapeshifterMoves?.solver?.type === 'searching', null, { timeout: 60000 });
                const info = await page.evaluate(() => shapeshifterMoves.solver);
                assert(await badge.isVisible());
                assert.equal(await badge.textContent(), `${info.threaded ? 'SIMD' : 'Scalar'} · ${info.workers} worker${info.workers === 1 ? '' : 's'}`);
                assert((await badge.getAttribute('title')).includes('JIT optimization cannot be verified'));
                assert(await page.locator('#shapeshifter-status .solver-message').isHidden(), 'Normal progress produces no warning');
                assert.equal(await progress.getAttribute('role'), 'timer');
                assert.match(await progress.textContent(), /^Searching · (2:00|1:59) remaining$/);
                // A delayed/background timer must catch up to elapsed time, not just subtract one tick.
                await page.clock.fastForward(65000);
                assert.match(await progress.textContent(), /^Searching · 0:5[45] remaining$/);
                await page.locator('#shapeshifter-status button').click();
            }
            async function assertPlacementProgress(total) {
                await page.waitForFunction(() => window.shapeshifterMoves?.phase === 'placing' && shapeshifterMoves.completed === 1);
                assert.equal(await progress.textContent(), `Placing pieces · 1 / ${total}`);
                assert.equal(await page.locator('.solver-copy-debug').count(), 0, 'Debug copying is only shown for errors');
                assert(await badge.isVisible(), 'Keep solver capabilities visible after the search workers finish');
                assert.equal(await page.locator('#shapeshifter-status [role="timer"]').count(), 0, 'Stop the countdown during placement');
            }
            async function assertSolved() {
                await page.waitForFunction(() => document.body.textContent.includes('You Won!')
                    || window.shapeshifterMoves?.error, null, { timeout: 60000 });
                assert((await page.locator('body').innerText()).includes('You Won!'), logs.join('\n'));
                assert.equal(moves, scenario === 'timeout-win' ? TIMEOUT_PUZZLE.pieces.length
                    : puzzle.pieces.length + (timeout ? TIMEOUT_PUZZLE.pieces.length : 0));
                assert.equal(requests.length, moves);
                assert(state.board.every(row => row.every(value => value === 0)), 'Replay solves the original board');
                for (let i = 1; i < requests.length; i++) {
                    assert(requests[i].at - requests[i - 1].at >= 900, 'Wait between confirmed moves');
                }
            }
            if (['success', 'start-lost', 'timeout-restart', 'timeout-win'].includes(scenario)) {
                if (hostedUrl && scenario === 'success') await assertPlacementProgress(puzzle.pieces.length);
                if (scenario === 'timeout-restart') {
                    await assertPlacementProgress(TIMEOUT_PUZZLE.pieces.length);
                    await page.waitForFunction(() => window.shapeshifterMoves?.phase === 'loading' && shapeshifterMoves.result?.timed_out);
                    assert(await badge.isVisible(), 'Keep the last search capabilities visible while starting a new puzzle');
                    assert.equal(await progress.textContent(), 'Loading solver…');
                    assert.equal(await page.evaluate(() => shapeshifterMoves.completed), 0);
                    await assertPlacementProgress(puzzle.pieces.length);
                }
                await assertSolved();
                assert.equal(restarts, ['start-lost', 'timeout-restart'].includes(scenario) ? 1 : 0);
                if (timeout) assert(logs.some(text => text.includes('Search timed out.')));
            } else {
                if (['stop', 'timeout-stop'].includes(scenario)) {
                    if (hostedUrl) await assertPlacementProgress(timeout ? TIMEOUT_PUZZLE.pieces.length : puzzle.pieces.length);
                    else await page.waitForFunction(() => window.shapeshifterMoves?.completed === 1);
                    if (timeout) await page.locator('#shapeshifter-status button').click();
                    else await page.evaluate(() => window.shapeshifterMoves.stop());
                }
                await page.waitForFunction(() => window.shapeshifterMoves?.running === false);
                assert.equal(requests.length, ['wrong-board', 'changed-before-moves', 'solver-error', 'timeout-changed', 'cancel-search'].includes(scenario) ? 0
                    : ['timeout-bad-form', 'timeout-restart-error', 'unexpected-final'].includes(scenario) ? TIMEOUT_PUZZLE.pieces.length : 1,
                    'Stop without retrying or placing more pieces');
                assert.equal(restarts, scenario === 'timeout-restart-error' ? 1 : 0);
                const control = await page.evaluate(() => ({ message: shapeshifterMoves.message, result: shapeshifterMoves.result,
                    error: shapeshifterMoves.error, stopped: shapeshifterMoves.stopped }));
                if (control.error) assert(logs.some(text => text.includes(control.error)), logs.join('\n'));
                if (hostedUrl) {
                    if (timeout) {
                        assert(control.result.timed_out && !control.result.solved);
                        assert(logs.some(text => text.includes('Search timed out.')));
                    }
                    if (scenario === 'timeout-bad-form') assert(control.error.includes('Try Again form'));
                    if (scenario === 'timeout-restart-error') assert(control.error.includes('HTTP 500'));
                    if (scenario === 'solver-error') assert(control.error.includes('Test solver failed'));
                    if (control.error) {
                        assert.equal(await page.locator('#shapeshifter-status .solver-message').textContent(), control.message);
                        assert(await progress.isHidden(), 'Do not leave stale progress running after an error');
                        assert(await page.locator('#shapeshifter-status').isVisible(), 'Keep errors visible without DevTools');
                        const copy = page.locator('#shapeshifter-status .solver-copy-debug');
                        const activity = { reads, moves, restarts };
                        if (scenario === 'unexpected-page') {
                            await page.evaluate(() => { navigator.clipboard.writeText = async () => { throw new Error('Clipboard denied'); }; });
                            await copy.click();
                            await page.waitForFunction(() => document.querySelector('.solver-copy-debug').textContent === 'Copy failed');
                        }
                        await page.evaluate(() => { navigator.clipboard.writeText = async text => { window.copiedDebug = text; }; });
                        await copy.click();
                        await page.waitForFunction(() => document.querySelector('.solver-copy-debug').textContent === 'Copied!');
                        const debug = JSON.parse(await page.evaluate(() => window.copiedDebug));
                        assert.equal(debug.runnerUrl, `${hostedUrl}/console-runner.js`);
                        assert.equal(debug.pageUrl, GAME);
                        assert.equal(debug.error.message, control.error);
                        assert.equal(typeof debug.error.stack, 'string');
                        assert(!JSON.stringify(debug).includes('test_session=fixture'), 'Do not collect request cookies');
                        if (scenario.startsWith('unexpected-')) {
                            assert.equal(debug.request.html, UNEXPECTED_HTML, 'Copy the actual failing response, not the visible game page');
                            assert.equal(debug.request.status, 200);
                            assert.equal(debug.phase, 'placing');
                            assert.equal(debug.completed, moves - 1, 'Do not count the unconfirmed move');
                            assert.equal(debug.lastMove.piece, moves);
                            assert.deepEqual(debug.expectedBoard, state.board);
                        } else if (scenario === 'network-error') {
                            assert.equal(debug.request.html, null, 'Do not attribute a previous page to a failed request');
                            assert.equal(debug.request.status, null);
                            assert.equal(debug.completed, 0);
                            assert.equal(debug.lastMove.piece, 1);
                        } else if (scenario === 'timeout-restart-error') {
                            assert.equal(debug.request.status, 500);
                            assert.equal(debug.request.method, 'POST');
                            assert.equal(debug.request.html, 'The new game started, but its response failed.');
                        } else {
                            assert.equal(debug.request.html, gameHtml());
                        }
                        if (scenario.startsWith('unexpected-') || scenario === 'network-error') {
                            assert.equal(debug.request.url, requests.at(-1).url);
                        }
                        assert.deepEqual({ reads, moves, restarts }, activity, 'Copying debug info must not fetch or replay a request');
                        await page.locator('#shapeshifter-status button', { hasText: 'Dismiss' }).click();
                    } else assert(control.stopped, 'User cancellation is silent');
                    assert.equal(await page.locator('#shapeshifter-status').count(), 0);
                    assert.equal(await page.evaluate(() => shapeshifterMoves.message), control.message, 'Retain diagnostics after dismissal');
                }
                if (scenario === 'refused') assert(logs.some(text => text.includes('wrong place')));
                if (scenario === 'stop') {
                    await page.reload();
                    await page.evaluate(source => { (0, eval)(source); }, script);
                    await assertSolved();
                }
            }
            assert(requests.every(request => request.overlap === 0), 'Requests must be sequential');
            assert(logs.some(text => text.includes('already running')), 'Repeated pastes do not start another runner');
            assert.deepEqual(routineLogs, [], 'Only problems produce console output');
            assert.deepEqual(scenario === 'solver-error'
                ? errors.filter(error => !error.includes('Test solver failed')) : errors, []);
            assert.equal(await page.locator('iframe').count(), 0, 'Release the hosted solver after completion or cancellation');
        } catch (error) {
            throw new Error(`Scenario: ${scenario}\n${logs.join('\n')}`, { cause: error });
        } finally {
            await context.close();
        }
    }
    console.log(hostedUrl ? 'PASS console auto-solve: hosted workers, countdown, placement progress, replay, timeout forfeits and restart, live-state checks, copyable error details, cancellation, stop/resume'
        : 'PASS console move script: same-origin requests, live links, replay, delays, state checks, stop/resume, no retries');
}
