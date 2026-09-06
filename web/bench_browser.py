#!/usr/bin/env python3
"""Benchmark one JSONL puzzle at a time; preparation is outside the search cap."""
import argparse
import asyncio
import json
import time
from pathlib import Path

from playwright.async_api import async_playwright
from test_browser import open_client, serve, verify_solution


async def main(args):
    puzzles = [json.loads(line) for line in args.puzzles.read_text().splitlines() if line.strip()]
    with serve() as url, args.output.open('w') as output:
        async with async_playwright() as playwright:
            browser = await getattr(playwright, args.browser).launch()
            page, info = await open_client(browser, url, args.threads)
            for index, puzzle in enumerate(puzzles):
                start = time.monotonic()
                record = {'index': index, 'level': puzzle['level'], 'browser': args.browser,
                          'version': browser.version, **info}
                if 'seed' in puzzle:
                    record['seed'] = puzzle['seed']
                try:
                    result = await asyncio.wait_for(page.evaluate(
                        '(p) => client.solve(p.puzzle, p.budget)',
                        {'puzzle': puzzle, 'budget': args.budget * 1000}), args.budget + 180)
                    if result['solved']:
                        verify_solution(puzzle, result)
                    record.update(result)
                except Exception as error:
                    record['error'] = str(error)
                    await page.close()
                    page, info = await open_client(browser, url, args.threads)
                record['wall_ms'] = (time.monotonic() - start) * 1000
                output.write(json.dumps(record) + '\n')
                output.flush()
                print(json.dumps({k: v for k, v in record.items() if k != 'placements'}), flush=True)
            await browser.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('puzzles', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--threads', type=int, help='Default: all browser-reported CPUs')
    parser.add_argument('--budget', type=int, default=120)
    parser.add_argument('--browser', choices=['chromium', 'firefox', 'webkit'], default='chromium')
    asyncio.run(main(parser.parse_args()))
