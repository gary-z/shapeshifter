import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parseArgs } from 'node:util';
import * as playwright from 'playwright';
import { serve } from '../../scripts/serve.mjs';
import { openClient, verifySolution } from '../../scripts/browser-tools.mjs';
import { testConsoleSolver } from './console-solver.mjs';

const PREPARATION_TIMEOUT_MS = 180_000;
const easy = JSON.parse(readFileSync(new URL('./fixtures/easy.json', import.meta.url), 'utf8'));
const hard = JSON.parse(readFileSync(new URL('./fixtures/hard.json', import.meta.url), 'utf8'));

export async function testMode(browser, isolated, deployedUrl) {
    const server = deployedUrl ? { url: deployedUrl, close: async () => {} } : await serve({ isolated });
    try {
        const { page, info } = await openClient(browser, server.url, 4);
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

        if (isolated) await testConsoleSolver(browser, server.url, easy, hard);
    } finally {
        await server.close();
    }
    console.log(`PASS ${isolated ? 'shared-memory' : 'single-worker'}: solution replay, reuse, validation, ` +
        'deadline, cancellation, responsiveness');
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
