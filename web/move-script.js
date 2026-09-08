import { parseShapeshifterHtml } from './parser.js';

// Serialized into the console script; keep this function self-contained.
async function runMoves(puzzle, placements, parseHtml, delayMs) {
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
        running: true, stopped: false, completed: 0,
        stop() { this.stopped = true; },
    };
    const gameUrl = new URL('/medieval/shapeshifter.phtml', location.origin);
    const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    const expected = puzzle.board.map(row => [...row]);
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
    async function read(url) {
        const response = await fetch(url, {
            mode: 'same-origin', credentials: 'same-origin', cache: 'no-store',
            referrer: gameUrl.href, referrerPolicy: 'same-origin',
            signal: AbortSignal.timeout(30000),
        });
        if (!response.ok) throw new Error(`Neopets returned HTTP ${response.status}.`);
        const html = await response.text();
        if (/from the wrong place/i.test(html)) throw new Error('Neopets rejected the request: wrong place.');
        return html;
    }
    try {
        console.log(`Checking the live puzzle. Delay between moves: ${delayMs} ms. Stop with shapeshifterMoves.stop().`);
        let html = await read(gameUrl);
        const current = parseHtml(html);
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
            console.log(`Placing piece ${index + 1}/${placements.length} at row ${row}, column ${col}…`);
            html = await read(url);
            apply(index);
            const last = index + 1 === placements.length;
            if (last) {
                if (!html.includes('You Won!') || Number(html.match(/LEVEL\s+(\d+)/)?.[1]) !== puzzle.level) {
                    throw new Error('The final response did not confirm that this level was solved.');
                }
            } else {
                check(parseHtml(html), index + 1);
            }
            control.completed = index + 1;
            console.log(`Confirmed ${control.completed}/${placements.length} pieces.`);
            if (last) {
                console.log('Neopets confirmed the level is solved. Refreshing the game page.');
                location.assign(gameUrl.href);
                return;
            }
            if (!control.stopped) await new Promise(resolve => setTimeout(resolve, delayMs));
        }
        console.log('Move script stopped. Refresh the game page before continuing.');
    } catch (error) {
        console.error(`Move script stopped: ${error.message} No move was retried. Refresh the game page before continuing.`);
    } finally {
        control.running = false;
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
    const button = document.createElement('button');
    button.textContent = 'Copy move script';
    button.className = 'copy-move-script';
    container.querySelector('.step-nav').append(button);

    const details = document.createElement('details');
    details.className = 'move-script-help';
    const summary = document.createElement('summary');
    summary.textContent = 'Run moves on Neopets (experimental)';
    const instructions = document.createElement('p');
    instructions.textContent = 'Copy the script, switch to your Neopets game tab, open the developer console, '
        + 'paste, and press Enter. It places the remaining pieces one at a time, waiting one second between '
        + 'confirmed moves, then refreshes the game page. Keep the game tab idle while it runs. '
        + 'To stop, enter shapeshifterMoves.stop() in that console.';
    const feedback = document.createElement('p');
    feedback.setAttribute('role', 'status');
    const source = document.createElement('textarea');
    source.className = 'move-script-source';
    source.setAttribute('aria-label', 'Move script for the Neopets console');
    source.readOnly = true;
    source.value = createMoveScript(puzzle, placements);
    details.append(summary, instructions, feedback, source);
    container.querySelector('.step-nav').after(details);
    button.addEventListener('click', async () => {
        details.open = true;
        try {
            await navigator.clipboard.writeText(source.value);
            feedback.textContent = 'Copied. Paste into the console on your Neopets game tab.';
        } catch {
            source.focus();
            source.select();
            feedback.textContent = 'Copy the selected script below, then paste into the console on your Neopets game tab.';
        }
    });
}
