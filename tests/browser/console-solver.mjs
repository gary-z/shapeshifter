import assert from 'node:assert/strict';
import { testMoveScript } from './move-script.mjs';

export async function testConsoleSolver(browser, url, easy, hard) {
    const page = await browser.newPage();
    const requests = [];
    const errors = [];
    page.on('request', request => requests.push(request.url()));
    page.on('pageerror', error => errors.push(String(error)));
    let script;
    try {
        const response = await page.goto(`${url}/`);
        assert(response.ok());
        await page.clock.install();
        const copy = page.getByRole('button', { name: 'Copy auto-solve script' });
        assert(await copy.isVisible());
        assert.equal(await page.locator('button').count(), 1);
        assert.equal(await page.locator('textarea, input, .board, iframe').count(), 0);
        await page.evaluate(() => {
            navigator.clipboard.writeText = async text => { window.copiedScript = text; };
        });
        await copy.click();
        await page.waitForFunction(() => !!window.copiedScript);
        script = await page.evaluate(() => window.copiedScript);
        assert(script.includes(`${url}/console-runner.js`));
        assert.equal(await page.locator('#copy-console-script').textContent(), 'Copied!');
        await page.clock.fastForward(2000);
        assert(await copy.isVisible());
        await page.evaluate(() => {
            navigator.clipboard.writeText = async () => { throw new Error('Clipboard denied'); };
        });
        await copy.click();
        await page.waitForFunction(() => document.querySelector('#copy-console-script').textContent === 'Copy failed');
        await page.clock.fastForward(2000);
        assert(await copy.isVisible());

        assert.equal(page.workers().length, 0, 'Opening the page must not start solver workers');
        assert(!requests.some(request => /\.(wasm)(\?|$)|\/(search-worker|console-runner|console-solver)\./.test(request)),
            'Opening and copying must not load the solver');

        await Promise.all([
            page.waitForEvent('console', { predicate: entry => entry.text().includes('Neopets Shapeshifter game tab') }),
            page.evaluate(source => { (0, eval)(source); }, script),
        ]);
        assert.equal(await page.locator('#shapeshifter-status').count(), 0, 'A misplaced paste must not start a solve');
        assert.deepEqual(errors, []);
    } finally {
        await page.close();
    }
    console.log('PASS console page: copy, clipboard errors, no idle workers, wrong-page guard');
    await testMoveScript(browser, script, easy, { hostedUrl: url, hard });
}
