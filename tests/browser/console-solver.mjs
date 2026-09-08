import assert from 'node:assert/strict';
import { testMoveScript } from './move-script.mjs';

export async function testConsoleSolver(browser, url, easy, hard) {
    const page = await browser.newPage();
    let script;
    try {
        const response = await page.goto(`${url}/console.html`);
        assert(response.ok());
        await page.evaluate(() => {
            navigator.clipboard.writeText = async text => { window.copiedScript = text; };
        });
        await page.locator('#copy-console-script').click();
        await page.waitForFunction(() => !!window.copiedScript);
        script = await page.evaluate(() => window.copiedScript);
        assert(script.includes(`${url}/console-runner.js`));
        assert.equal(await page.locator('textarea, input').count(), 0);
    } finally {
        await page.close();
    }
    await testMoveScript(browser, script, easy, { hostedUrl: url, hard });
}
