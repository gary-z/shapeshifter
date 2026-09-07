import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { parseArgs } from 'node:util';
import * as playwright from 'playwright';
import { ROOT, serve } from './serve.mjs';
import { openClient, puzzleHtml, verifySolution } from './browser-tools.mjs';

const PREPARATION_TIMEOUT_MS = 180_000;
const easy = JSON.parse(readFileSync(join(ROOT, 'web/tests/easy.json'), 'utf8'));
const hard = JSON.parse(readFileSync(join(ROOT, 'web/tests/hard.json'), 'utf8'));

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
        await page.goto(server.url);
        await page.waitForFunction(() => !document.getElementById('solve-btn').disabled, null, { timeout: 65000 });
        const expected = isolated ? await page.evaluate(() => navigator.hardwareConcurrency) : 1;
        assert((await page.locator('#status').innerText()).includes(`${expected} search worker`));
        await page.locator('#puzzle-input').fill('<img src=x onerror=alert(1)>');
        await page.locator('#solve-btn').click();
        assert((await page.locator('#results-content').innerText()).includes('Parse error:'));
        await page.locator('#puzzle-input').fill(puzzleHtml(easy));
        await page.locator('#solve-btn').click();
        await page.waitForFunction(() => !document.getElementById('solve-btn').disabled);
        assert.equal(await page.locator('.step-nav').count(), 1, await page.locator('#results-content').innerText());
        assert((await page.locator('#status').innerText()).includes('search'));
        await page.locator('#puzzle-input').fill(puzzleHtml(hard));
        await page.locator('#solve-btn').click();
        await page.waitForFunction(() => document.getElementById('status').textContent.startsWith('Searching'),
            null, { timeout: PREPARATION_TIMEOUT_MS });
        await page.locator('#cancel-btn').click();
        await page.waitForFunction(() => !document.getElementById('solve-btn').disabled);
        assert((await page.locator('#results-content').innerText()).includes('cancelled'));
        assert.deepEqual(errors, []);
        await page.close();
    } finally {
        await server.close();
    }
    console.log(`PASS ${isolated ? 'shared-memory' : 'single-worker'}: solution replay, reuse, validation, ` +
        'deadline, cancellation, responsiveness, UI');
}

async function testStartupFallback(browser) {
    const server = await serve({ failThreaded: true });
    try {
        const page = await browser.newPage();
        await page.goto(server.url + '/web/tests/easy.json');
        const result = await page.evaluate(async () => {
            const { SolverClient } = await import('/web/search-client.js');
            const client = new SolverClient({ threads: 4 });
            const first = await client.init();
            const sameWorker = client.worker;
            const second = await client.init();
            const reused = sameWorker === client.worker;
            client.dispose();
            return { first, second, reused };
        });
        assert.deepEqual(result, { first: { workers: 1, threaded: false },
            second: { workers: 1, threaded: false }, reused: true });
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
