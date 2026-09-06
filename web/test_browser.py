#!/usr/bin/env python3
"""Real-browser tests for both shared-memory and single-worker search."""
import argparse
import asyncio
import functools
import http.server
import json
import threading
from contextlib import contextmanager
from pathlib import Path

from playwright.async_api import async_playwright

ROOT = Path(__file__).resolve().parent.parent


@contextmanager
def serve(root=ROOT, isolated=True, fail_threaded=False):
    class Handler(http.server.SimpleHTTPRequestHandler):
        def do_GET(self):
            if fail_threaded and '/pkg-threaded/' in self.path:
                self.send_error(503, 'Test: parallel package unavailable')
            else:
                super().do_GET()

        def end_headers(self):
            if isolated:
                self.send_header('Cross-Origin-Opener-Policy', 'same-origin')
                self.send_header('Cross-Origin-Embedder-Policy', 'require-corp')
            super().end_headers()

        def log_message(self, *_args):
            pass

    server = http.server.ThreadingHTTPServer(
        ('127.0.0.1', 0), functools.partial(Handler, directory=str(root)))
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f'http://127.0.0.1:{server.server_port}'
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def verify_solution(puzzle, result):
    assert result['solved'], result
    placements = result['placements']
    assert len(placements) == len(puzzle['pieces'])
    board = [row[:] for row in puzzle['board']]
    for piece, (row, column) in zip(puzzle['pieces'], placements):
        assert 0 <= row <= puzzle['rows'] - len(piece)
        assert 0 <= column <= puzzle['columns'] - len(piece[0])
        for r, cells in enumerate(piece):
            for c, active in enumerate(cells):
                if active:
                    board[row+r][column+c] = (board[row+r][column+c] - 1) % puzzle['m']
    assert not any(value for row in board for value in row), 'Incorrect original-order placements'


def puzzle_html(puzzle):
    """Small page-source fixture using the game's board/piece markup."""
    icons = ['hel', 'swo', 'cro', 'gob', 'sta'][:puzzle['m']]
    lines = [f'LEVEL {puzzle["level"]}; gX = {puzzle["columns"]}; gY = {puzzle["rows"]};']
    for row, cells in enumerate(puzzle['board']):
        for column, value in enumerate(cells):
            lines.append(f'imgLocStr[{column}][{row}] = "{icons[value]}";')
    # Explicit cycle: the first icon is the goal, subsequent icons decrease values.
    cycle = [icons[0], *reversed(icons[1:])]
    lines.append('<table>' + ''.join(
        f'<img src="/{icon}_0.gif">' + ('<br><b><small>GOAL' if i == 0 else '')
        for i, icon in enumerate(cycle)) + '</table>')
    lines.append('ACTIVE SHAPE')
    for index, piece in enumerate(puzzle['pieces']):
        if index == 1:
            lines.append('NEXT SHAPES')
        lines.append('<table border=0 cellpadding=0 cellspacing=0>')
        for row in piece:
            lines.append('<tr>' + ''.join(
                '<td><img src="square.gif"></td>' if cell else '<td></td>' for cell in row) + '</tr>')
        lines.append('</table>')
    return '\n'.join(lines)


async def open_client(browser, url, threads=None):
    page = await browser.new_page()
    # A blank document on the same origin, without starting the product UI's pool.
    await page.goto(url + '/web/tests/easy.json')
    info = await page.evaluate('''async (threads) => {
        const { SolverClient } = await import('/web/search-client.js');
        window.events = [];
        window.client = new SolverClient({
            ...(threads === null ? {} : { threads }),
            onStatus: event => events.push(event),
        });
        return await client.init();
    }''', threads)
    return page, info


async def test_mode(browser, isolated):
    easy = json.loads((ROOT / 'web/tests/easy.json').read_text())
    hard = json.loads((ROOT / 'web/tests/hard.json').read_text())
    with serve(isolated=isolated) as url:
        page, info = await open_client(browser, url, threads=4)
        assert info == {'threaded': isolated, 'workers': 4 if isolated else 1}, info
        for _ in range(2):
            verify_solution(easy, await page.evaluate('p => client.solve(p)', easy))
        invalid = {**easy, 'm': 0}
        error = await page.evaluate('p => client.solve(p).then(() => null, e => e.message)', invalid)
        assert error and 'Invalid' in error, error
        verify_solution(easy, await page.evaluate('p => client.solve(p)', easy))
        result = await page.evaluate('p => client.solve(p, 100)', hard)
        assert result['timed_out'] and not result['solved'], result
        assert 90 <= result['search_ms'] < 2000, result
        # Cancellation must be possible while Rust occupies its worker(s).
        await page.evaluate('''p => {
            window.ticks = 0;
            window.timer = setInterval(() => ticks++, 10);
            window.events = [];
            window.job = client.solve(p);
        }''', hard)
        await page.wait_for_function("events.some(e => e.type === 'searching')")
        await page.wait_for_timeout(200)
        assert await page.evaluate('ticks') >= 5, 'Main thread froze during search'
        busy = await page.evaluate('p => client.solve(p).then(() => null, e => e.message)', easy)
        assert 'already running' in busy
        result = await page.evaluate('async () => { client.cancel(); return await job; }')
        assert result['cancelled'] and not result['solved'], result
        await page.evaluate('clearInterval(timer)')
        verify_solution(easy, await page.evaluate('p => client.solve(p)', easy))
        await page.evaluate('client.dispose()')
        # Cancellation while the pool is still loading must settle immediately,
        # and a late rejection from that startup must not kill the next pool.
        result = await page.evaluate('''async (p) => {
            const pending = client.solve(p);
            client.cancel();
            return await pending;
        }''', hard)
        assert result['cancelled'], result
        verify_solution(easy, await page.evaluate('p => client.solve(p)', easy))
        await page.evaluate('client.dispose()')
        await page.close()
        # Exercise the public UI and its default use of all browser-reported CPUs.
        page = await browser.new_page()
        errors = []
        page.on('pageerror', lambda error: errors.append(str(error)))
        await page.goto(url)
        await page.wait_for_function("!document.getElementById('solve-btn').disabled", timeout=65000)
        expected = await page.evaluate('navigator.hardwareConcurrency') if isolated else 1
        assert f'{expected} search worker' in await page.locator('#status').inner_text()
        await page.locator('#puzzle-input').fill('<img src=x onerror=alert(1)>')
        await page.locator('#solve-btn').click()
        assert 'Parse error:' in await page.locator('#results-content').inner_text()
        await page.locator('#puzzle-input').fill(puzzle_html(easy))
        await page.locator('#solve-btn').click()
        await page.wait_for_function("!document.getElementById('solve-btn').disabled")
        assert await page.locator('.step-nav').count() == 1, await page.locator('#results-content').inner_text()
        assert 'search' in await page.locator('#status').inner_text()
        await page.locator('#puzzle-input').fill(puzzle_html(hard))
        await page.locator('#solve-btn').click()
        await page.wait_for_function("document.getElementById('status').textContent.startsWith('Searching')")
        await page.locator('#cancel-btn').click()
        await page.wait_for_function("!document.getElementById('solve-btn').disabled")
        assert 'cancelled' in await page.locator('#results-content').inner_text()
        assert not errors, errors
        await page.close()
    print(f'PASS {"shared-memory" if isolated else "single-worker"}: solution replay, reuse, validation, deadline, cancellation, responsiveness, UI', flush=True)


async def test_startup_fallback(browser):
    with serve(fail_threaded=True) as url:
        page = await browser.new_page()
        await page.goto(url + '/web/tests/easy.json')
        result = await page.evaluate('''async () => {
            const { SolverClient } = await import('/web/search-client.js');
            const client = new SolverClient({threads: 4});
            const first = await client.init();
            const sameWorker = client.worker;
            const second = await client.init();
            const reused = sameWorker === client.worker;
            client.dispose();
            return {first, second, reused};
        }''')
        assert result == {'first': {'workers': 1, 'threaded': False},
                          'second': {'workers': 1, 'threaded': False}, 'reused': True}, result
        await page.close()
    print('PASS failed parallel startup falls back and reuses the worker', flush=True)


async def main(browser_name):
    async with async_playwright() as playwright:
        browser = await getattr(playwright, browser_name).launch()
        try:
            await test_mode(browser, True)
            await test_mode(browser, False)
            await test_startup_fallback(browser)
        finally:
            await browser.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser', choices=['chromium', 'firefox', 'webkit'], default='chromium')
    asyncio.run(main(parser.parse_args().browser))
