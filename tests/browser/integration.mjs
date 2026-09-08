import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parseArgs } from 'node:util';
import * as playwright from 'playwright';
import { serve } from '../../scripts/serve.mjs';
import { openClient, puzzleHtml, verifySolution } from '../../scripts/browser-tools.mjs';
import { testMoveScript } from './move-script.mjs';
import { testConsoleSolver } from './console-solver.mjs';

const PREPARATION_TIMEOUT_MS = 180_000;
const easy = JSON.parse(readFileSync(new URL('./fixtures/easy.json', import.meta.url), 'utf8'));
const hard = JSON.parse(readFileSync(new URL('./fixtures/hard.json', import.meta.url), 'utf8'));

async function assertPreview(page, puzzle) {
    const icons = ['hel', 'swo', 'cro', 'gob', 'sta'];
    assert.equal(await page.locator('.step-nav').count(), 0);
    assert.equal(await page.locator('.cell').count(), puzzle.rows * puzzle.columns);
    assert.deepEqual(await page.locator('.cell img').evaluateAll(images => images.map(image =>
        new URL(image.src).pathname.split('/').pop())),
    puzzle.board.flat().map(value => `${icons[value]}_0.gif`));
    assert(await page.locator('#solve-btn').isVisible());
    assert.equal(await page.locator('#puzzle-input').getAttribute('aria-invalid'), 'false');
}

export async function testMode(browser, isolated, deployedUrl) {
    const server = deployedUrl ? { url: deployedUrl, close: async () => {} } : await serve({ isolated });
    try {
        let { page, info } = await openClient(browser, server.url, 4);
        assert.deepEqual(info, { threaded: isolated, workers: isolated ? 4 : 1 });
        for (let i = 0; i < 2; i++) verifySolution(easy, await page.evaluate(p => client.solve(p), easy));
        const error = await page.evaluate(p => client.solve(p).then(() => null, e => e.message), { ...easy, m: 0 });
        assert(error?.includes('Invalid'), error);
        verifySolution(easy, await page.evaluate(p => client.solve(p), easy));
        let result = await page.evaluate(p => client.solve(p, 100), hard);
        assert(result.timed_out && !result.solved, JSON.stringify(result));
        assert(result.search_ms >= 90 && result.search_ms < 2000, JSON.stringify(result));
        console.log(`Deadline check (${isolated ? 'shared-memory' : 'single-worker'}): ` +
            `${result.preparation_ms.toFixed(0)}ms preparation, ${result.search_ms.toFixed(0)}ms search`);
        await page.evaluate(p => {
            window.ticks = 0;
            window.timer = setInterval(() => ticks++, 10);
            window.events = [];
            window.job = client.solve(p);
        }, hard);
        await page.waitForFunction(() => events.some(e => e.type === 'searching'), null,
            { timeout: PREPARATION_TIMEOUT_MS });
        await page.waitForTimeout(200);
        assert(await page.evaluate(() => ticks) >= 5, 'Main thread froze during search');
        const busy = await page.evaluate(p => client.solve(p).then(() => null, e => e.message), easy);
        assert(busy.includes('already running'));
        result = await page.evaluate(async () => { client.cancel(); return await job; });
        assert(result.cancelled && !result.solved, JSON.stringify(result));
        await page.evaluate(() => clearInterval(timer));
        verifySolution(easy, await page.evaluate(p => client.solve(p), easy));
        await page.evaluate(() => client.dispose());
        result = await page.evaluate(async p => {
            const pending = client.solve(p);
            client.cancel();
            return await pending;
        }, hard);
        assert(result.cancelled, JSON.stringify(result));
        verifySolution(easy, await page.evaluate(p => client.solve(p), easy));
        await page.evaluate(() => client.dispose());
        await page.close();

        page = await browser.newPage();
        const errors = [];
        page.on('pageerror', error => errors.push(String(error)));
        const input = page.locator('#puzzle-input');
        const solve = page.locator('#solve-btn');
        const html = puzzleHtml(easy);
        let resumeStartup;
        const startupPaused = new Promise(resolve => { resumeStartup = resolve; });
        await page.route('**/search-worker.js', async route => {
            await startupPaused;
            await route.continue();
        });
        try {
            await page.goto(server.url, { waitUntil: 'domcontentloaded' });
            await page.waitForFunction(() => document.getElementById('status').textContent.includes('Loading'));
            assert(await solve.isHidden(), 'Solve is hidden before input');
            await input.fill(html);
            await assertPreview(page, easy);
            assert(await solve.isDisabled(), 'Preview does not wait for solver startup');

            for (const [invalid, expectedError] of [
                ['<img src=x onerror=alert(1)>', 'board dimensions'],
                ['You Won!', 'You Won!'],
                [html.replace('gX = 3', 'gX = 0'), 'Board dimensions'],
                [html.replace(/^imgLocStr\[0\]\[0\].*$/m, ''), 'every board cell'],
                [html.split('ACTIVE SHAPE')[0], 'any pieces'],
                [html.replaceAll('square.gif', 'empty.gif'), 'valid piece shapes'],
            ]) {
                await input.fill(invalid);
                const message = await page.locator('#results-content').innerText();
                assert(message.startsWith('Parse error:') && message.includes(expectedError), message);
                assert(await solve.isHidden());
                assert.equal(await page.locator('.board').count(), 0, 'Old board is removed on error');
                assert.equal(await input.getAttribute('aria-invalid'), 'true');
            }
            await input.fill('');
            assert.equal(await page.locator('.cell').count(), 36);
            assert(!(await page.locator('#results-content').innerText()).includes('Parse error:'));
            assert(await solve.isHidden());
        } finally {
            resumeStartup();
        }
        await page.waitForFunction(() => document.getElementById('status').textContent.startsWith('Ready'),
            null, { timeout: 65000 });
        assert(await solve.isHidden(), 'Worker readiness must not reveal Solve without input');
        assert(await solve.isDisabled());
        const expected = isolated ? await page.evaluate(() => navigator.hardwareConcurrency) : 1;
        assert((await page.locator('#status').innerText()).includes(`${expected} search worker`));
        await input.fill(html);
        await assertPreview(page, easy);
        await solve.click();
        await page.waitForFunction(() => !document.getElementById('solve-btn').disabled);
        assert.equal(await page.locator('.step-nav').count(), 1, await page.locator('#results-content').innerText());
        await page.waitForFunction(() => [...document.querySelectorAll('.cell img')]
            .every(image => image.complete && image.naturalWidth > 0));
        assert((await page.locator('#status').innerText()).includes('search'));
        await page.evaluate(() => {
            navigator.clipboard.writeText = async text => { window.copiedScript = text; };
        });
        const copy = page.locator('.copy-move-script');
        await copy.click();
        await page.waitForFunction(() => document.querySelector('.copy-move-script').textContent === 'Copied!');
        const moveScript = await page.evaluate(() => window.copiedScript);
        assert(moveScript.includes('shapeshifterMoves.stop()'));
        await page.evaluate(() => {
            navigator.clipboard.writeText = async () => { throw new Error('Clipboard denied'); };
        });
        await copy.click();
        await page.waitForFunction(() => document.querySelector('.copy-move-script').textContent === 'Copy failed');
        await page.waitForFunction(() => document.querySelector('.copy-move-script').textContent === 'Copy move script');
        if (isolated) {
            await testMoveScript(browser, moveScript, easy);
            await testConsoleSolver(browser, server.url, easy, hard);
        }
        await input.fill(puzzleHtml(hard));
        await assertPreview(page, hard);
        assert.equal(await page.locator('.copy-move-script').count(), 0, 'Remove the old solution script on new input');
        await solve.click();
        await page.waitForFunction(() => document.getElementById('status').textContent.startsWith('Searching'),
            null, { timeout: PREPARATION_TIMEOUT_MS });
        assert.equal(await page.locator('.cell').count(), hard.rows * hard.columns,
            'Board remains visible during search');
        await page.locator('#cancel-btn').click();
        await page.waitForFunction(() => !document.getElementById('solve-btn').disabled);
        assert((await page.locator('#results-content').innerText()).includes('cancelled'));
        await input.fill('   ');
        assert(await solve.isHidden());
        assert.equal(await page.locator('.cell').count(), 36);
        assert.deepEqual(errors, []);
        await page.close();
    } finally {
        await server.close();
    }
    console.log(`PASS ${isolated ? 'shared-memory' : 'single-worker'}: solution replay, reuse, validation, ` +
        'deadline, cancellation, responsiveness, paste preview and errors');
}

async function testStartupFallback(browser) {
    const server = await serve({ failThreaded: true });
    try {
        const { page, info } = await openClient(browser, server.url, 4);
        assert.deepEqual(info, { workers: 1, threaded: false });
        const result = await page.evaluate(async () => {
            const sameWorker = client.worker;
            const second = await client.init();
            const reused = sameWorker === client.worker;
            client.dispose();
            return { second, reused };
        });
        assert.deepEqual(result, { second: { workers: 1, threaded: false }, reused: true });
        await page.close();
    } finally {
        await server.close();
    }
    console.log('PASS failed parallel startup falls back and reuses the worker');
}

if (import.meta.main) {
    const { values } = parseArgs({ options: {
        browser: { type: 'string', default: 'all' }, url: { type: 'string' },
        'executable-path': { type: 'string' }, help: { type: 'boolean', short: 'h' },
    } });
    if (values.help) {
        console.log('Usage: npm run test:browser -- [--browser chromium|firefox|webkit] [--url URL] [--executable-path PATH]');
    } else {
        assert(['all', 'chromium', 'firefox', 'webkit'].includes(values.browser), 'Invalid browser');
        assert(!values['executable-path'] || values.browser !== 'all', 'Select a browser for --executable-path');
        for (const name of values.browser === 'all' ? ['chromium', 'firefox'] : [values.browser]) {
            const browser = await playwright[name].launch({ executablePath: values['executable-path'] });
            console.log(`Testing ${name} ${browser.version()}`);
            try {
                await testMode(browser, true, values.url?.replace(/\/$/, ''));
                if (!values.url) {
                    await testMode(browser, false);
                    await testStartupFallback(browser);
                }
            } finally {
                await browser.close();
            }
        }
    }
}
