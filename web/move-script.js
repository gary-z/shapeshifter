import { parseShapeshifterHtml } from './parser.js';
import { copyOnClick } from './copy-button.js';

// Serialized into the console script; keep this function self-contained.
export async function runMoves(puzzle, placements, parseHtml, delayMs, solve, onStatus) {
    if (location.protocol !== 'https:' || !['www.neopets.com', 'neopets.com'].includes(location.hostname)
        || location.pathname !== '/medieval/shapeshifter.phtml') {
        console.error('Run this script in the console of your Neopets Shapeshifter game tab.');
        return;
    }
    if (window.shapeshifterMoves?.running) {
        console.warn('A move script is already running. Use shapeshifterMoves.stop() to stop it.');
        return;
    }
    const control = window.shapeshifterMoves = {
        running: true, stopped: false, completed: 0, message: '', result: null, error: null,
        stop() { this.stopped = true; this.onStop?.(); },
        report(message, level = 'log') {
            this.message = message;
            console[level](message);
            onStatus?.(message, this);
        },
    };
    const gameUrl = new URL('/medieval/shapeshifter.phtml', location.origin);
    const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    let expected;
    function apply(index) {
        const [row, col] = placements[index];
        puzzle.pieces[index].forEach((cells, r) => cells.forEach((active, c) => {
            if (active) expected[row + r][col + c] = (expected[row + r][col + c] + puzzle.m - 1) % puzzle.m;
        }));
    }
    function check(current, index) {
        if (current.level !== puzzle.level || current.m !== puzzle.m
            || current.rows !== puzzle.rows || current.columns !== puzzle.columns
            || !same(current.board, expected) || !same(current.pieces, puzzle.pieces.slice(index))) {
            throw new Error('The live board or remaining pieces do not match this solution.');
        }
    }
    async function read(url, options = {}) {
        const response = await fetch(url, {
            ...options,
            mode: 'same-origin', credentials: 'same-origin', cache: 'no-store',
            referrer: gameUrl.href, referrerPolicy: 'same-origin',
            signal: AbortSignal.timeout(30000),
        });
        if (!response.ok) throw new Error(`Neopets returned HTTP ${response.status}.`);
        const html = await response.text();
        if (/from the wrong place/i.test(html)) throw new Error('Neopets rejected the request: wrong place.');
        return html;
    }
    async function restart(html) {
        const level = Number(html.match(/LEVEL\s+(\d+)/)?.[1]);
        const page = new DOMParser().parseFromString(html, 'text/html');
        const form = page.querySelector('form[name="start_game"]');
        const action = form?.getAttribute('action');
        const url = action && new URL(action, gameUrl);
        if (!url || url.origin !== gameUrl.origin || url.pathname !== '/medieval/process_shapeshifter.phtml'
            || url.searchParams.get('type') !== 'init' || form.method !== 'post') {
            throw new Error('Could not find the game’s Try Again form.');
        }
        control.report('Neopets confirmed the loss. Starting a new puzzle…');
        const next = await read(url, { method: 'POST', body: new URLSearchParams(new FormData(form)) });
        if (parseHtml(next).level !== level) throw new Error('Try Again returned a different level.');
        return next;
    }
    async function play(html, current, forfeiting) {
        expected = puzzle.board.map(row => [...row]);
        let index = puzzle.pieces.length - current.pieces.length;
        if (index < 0 || index >= placements.length) throw new Error('This solution does not match the live puzzle.');
        for (let i = 0; i < index; i++) apply(i);
        check(current, index);
        control.completed = index;
        for (; index < placements.length; index++) {
            if (control.stopped) break;
            const [row, col] = placements[index];
            const page = new DOMParser().parseFromString(html, 'text/html');
            const href = page.querySelector(`img[name="i${col}_${row}"]`)?.closest('a')?.getAttribute('href');
            const url = href && new URL(href, gameUrl);
            if (!url || url.origin !== gameUrl.origin || url.pathname !== '/medieval/process_shapeshifter.phtml'
                || url.searchParams.get('type') !== 'action'
                || url.searchParams.get('posx') !== String(col) || url.searchParams.get('posy') !== String(row)) {
                throw new Error(`Could not find the game's placement link at row ${row}, column ${col}.`);
            }
            control.report(`Placing piece ${index + 1}/${placements.length} at row ${row}, column ${col}…`);
            html = await read(url);
            apply(index);
            const last = index + 1 === placements.length;
            const won = html.includes('You Won!');
            if (last) {
                if (!(won || (forfeiting && html.includes('You Lost!')))
                    || Number(html.match(/LEVEL\s+(\d+)/)?.[1]) !== puzzle.level) {
                    throw new Error('The final response did not confirm the expected game result.');
                }
            } else {
                check(parseHtml(html), index + 1);
            }
            control.completed = index + 1;
            control.report(`Confirmed ${control.completed}/${placements.length} pieces.`);
            if (last) return { html, won };
            if (!control.stopped) await new Promise(resolve => setTimeout(resolve, delayMs));
        }
        return null;
    }
    try {
        control.report(`Checking the live puzzle. Delay between moves: ${delayMs} ms. Stop with shapeshifterMoves.stop().`);
        let html = await read(gameUrl);
        while (!control.stopped) {
            if (solve && html.includes('You Lost!')) {
                html = await restart(html);
                continue;
            }
            let current = parseHtml(html);
            let forfeiting = false;
            if (solve) {
                puzzle = current;
                control.completed = 0;
                const result = await solve(puzzle, control);
                control.result = result;
                if (control.stopped || result.cancelled) {
                    control.report('Solver stopped.');
                    return;
                }
                if (!result.solved && !result.timed_out) throw new Error('No solution found.');
                forfeiting = !result.solved;
                if (forfeiting) control.report('Search timed out. Placing the remaining pieces at the top left to start a new puzzle…');
                placements = forfeiting ? puzzle.pieces.map(() => [0, 0]) : result.placements;
                html = await read(gameUrl);
                current = parseHtml(html);
            }
            const outcome = await play(html, current, forfeiting);
            if (!outcome) break;
            if (outcome.won) {
                control.report('Neopets confirmed the level is solved. Refreshing the game page.');
                location.assign(gameUrl.href);
                return;
            }
            html = outcome.html;
        }
        control.report('Move script stopped. Refresh the game page before continuing.');
    } catch (error) {
        control.error = error.message;
        control.report(`Move script stopped: ${error.message} No move was retried. Refresh the game page before continuing.`, 'error');
    } finally {
        control.running = false;
        onStatus?.(control.message, control);
    }
}

export function createMoveScript(puzzle, placements, delayMs = 1000) {
    if (!Number.isFinite(delayMs) || delayMs < 0) throw new Error('Move delay must be a nonnegative number.');
    return `// Paste into the console on https://www.neopets.com/medieval/shapeshifter.phtml\n`
        + `// Stop: shapeshifterMoves.stop()\n`
        + `void (${runMoves.toString()})(\n${JSON.stringify(puzzle)},\n${JSON.stringify(placements)},\n`
        + `${parseShapeshifterHtml.toString()},\n${delayMs} /* milliseconds between confirmed moves */\n);`;
}

export function addMoveScript(container, puzzle, placements) {
    const script = createMoveScript(puzzle, placements);
    const button = document.createElement('button');
    button.textContent = 'Copy move script';
    button.className = 'copy-move-script';
    container.querySelector('.step-nav').append(button);
    copyOnClick(button, script);
}
