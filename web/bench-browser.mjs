import assert from 'node:assert/strict';
import { open } from 'node:fs/promises';
import { parseArgs } from 'node:util';
import * as playwright from 'playwright';
import { serve } from './serve.mjs';
import { openClient, readPuzzles, stringify, verifySolution } from './browser-tools.mjs';

const { values, positionals } = parseArgs({ allowPositionals: true, options: {
    output: { type: 'string' }, threads: { type: 'string' }, budget: { type: 'string', default: '120' },
    browser: { type: 'string', default: 'chromium' }, 'executable-path': { type: 'string' },
    help: { type: 'boolean', short: 'h' },
} });
if (values.help) {
    console.log('Usage: npm run bench:browser -- PUZZLES.jsonl --output RESULTS.jsonl ' +
        '[--threads N] [--budget 120] [--browser chromium|firefox|webkit] [--executable-path PATH]');
} else {
    assert(positionals.length === 1 && values.output, 'Provide a puzzle file and --output');
    assert(['chromium', 'firefox', 'webkit'].includes(values.browser), 'Invalid browser');
    const budget = Number(values.budget);
    const threads = values.threads === undefined ? null : Number(values.threads);
    assert(Number.isFinite(budget) && budget > 0, 'Invalid budget');
    assert(threads === null || (Number.isInteger(threads) && threads > 0), 'Invalid threads');
    const puzzles = readPuzzles(positionals[0]);
    const server = await serve();
    let browser, output;
    try {
        browser = await playwright[values.browser].launch({ executablePath: values['executable-path'] });
        output = await open(values.output, 'w');
        let { page, info } = await openClient(browser, server.url, threads);
        for (const [index, puzzle] of puzzles.entries()) {
            const start = performance.now();
            const { seed, ...game } = puzzle;
            const record = { index, level: puzzle.level, browser: values.browser,
                version: browser.version(), ...info, ...(seed === undefined ? {} : { seed }) };
            let watchdog;
            try {
                const result = await Promise.race([
                    page.evaluate(({ puzzle, budget }) => client.solve(puzzle, budget), { puzzle: game, budget: budget * 1000 }),
                    new Promise((_, reject) => { watchdog = setTimeout(() => reject(new Error('Browser solve watchdog expired')), (budget + 180) * 1000); }),
                ]);
                if (result.solved) verifySolution(puzzle, result);
                Object.assign(record, result);
            } catch (error) {
                record.error = String(error);
                await page.close();
                ({ page, info } = await openClient(browser, server.url, threads));
            } finally {
                clearTimeout(watchdog);
            }
            record.wall_ms = performance.now() - start;
            await output.write(stringify(record) + '\n');
            const { placements, ...summary } = record;
            console.log(stringify(summary));
        }
    } finally {
        await output?.close();
        await browser?.close();
        await server.close();
    }
}
