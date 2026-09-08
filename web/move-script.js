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
        running: true, stopped: false, completed: 0, total: 0, phase: 'reading', message: '', result: null, error: null,
        stop() { this.stopped = true; this.onStop?.(); onStatus?.(this.message, this); },
        update(state) { Object.assign(this, state); onStatus?.(this.message, this); },
        report(message, level = 'warn') {
            this.message = message;
            if (message) console[level](message);
            onStatus?.(message, this);
        },
    };
    const gameUrl = new URL('/medieval/shapeshifter.phtml', location.origin);
    const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    let expected, lastRequest = null, lastMove = null;
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
        lastRequest = { url: String(url), method: options.method ?? 'GET', responseUrl: null, status: null, html: null };
        const response = await fetch(url, {
            ...options,
            mode: 'same-origin', credentials: 'same-origin', cache: 'no-store',
            referrer: gameUrl.href, referrerPolicy: 'same-origin',
            signal: AbortSignal.timeout(30000),
        });
        lastRequest.responseUrl = response.url;
        lastRequest.status = response.status;
        const html = await response.text();
        lastRequest.html = html;
        if (!response.ok) throw new Error(`Neopets returned HTTP ${response.status}.`);
        if (/from the wrong place/i.test(html)) throw new Error('Neopets rejected the request: wrong place.');
        return html;
    }
    async function restart(html) {
        control.update({ phase: 'restarting', completed: 0, total: 0 });
        const level = Number(html.match(/LEVEL\s+(\d+)/)?.[1]);
        const page = new DOMParser().parseFromString(html, 'text/html');
        const form = page.querySelector('form[name="start_game"]');
        const action = form?.getAttribute('action');
        const url = action && new URL(action, gameUrl);
        if (!url || url.origin !== gameUrl.origin || url.pathname !== '/medieval/process_shapeshifter.phtml'
            || url.searchParams.get('type') !== 'init' || form.method !== 'post') {
            throw new Error('Could not find the game’s Try Again form.');
        }
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
        control.update({ phase: 'placing', completed: index, total: placements.length });
        for (; index < placements.length; index++) {
            if (control.stopped) break;
            const [row, col] = placements[index];
            lastMove = { piece: index + 1, row, col };
            const page = new DOMParser().parseFromString(html, 'text/html');
            const href = page.querySelector(`img[name="i${col}_${row}"]`)?.closest('a')?.getAttribute('href');
            const url = href && new URL(href, gameUrl);
            if (!url || url.origin !== gameUrl.origin || url.pathname !== '/medieval/process_shapeshifter.phtml'
                || url.searchParams.get('type') !== 'action'
                || url.searchParams.get('posx') !== String(col) || url.searchParams.get('posy') !== String(row)) {
                throw new Error(`Could not find the game's placement link at row ${row}, column ${col}.`);
            }
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
            control.update({ completed: index + 1 });
            if (last) return { html, won };
            if (!control.stopped) await new Promise(resolve => setTimeout(resolve, delayMs));
        }
        return null;
    }
    try {
        control.report('');
        let html = await read(gameUrl);
        while (!control.stopped) {
            control.report('');
            if (solve && html.includes('You Lost!')) {
                html = await restart(html);
                continue;
            }
            let current = parseHtml(html);
            let forfeiting = false;
            if (solve) {
                puzzle = current;
                placements = expected = lastMove = null;
                control.completed = 0;
                const result = await solve(puzzle, control);
                control.result = result;
                if (control.stopped || result.cancelled) {
                    return;
                }
                if (!result.solved && !result.timed_out) throw new Error('No solution found.');
                forfeiting = !result.solved;
                if (forfeiting) control.report('Search timed out. Starting a new puzzle.');
                placements = forfeiting ? puzzle.pieces.map(() => [0, 0]) : result.placements;
                html = await read(gameUrl);
                current = parseHtml(html);
            }
            const outcome = await play(html, current, forfeiting);
            if (!outcome) break;
            if (outcome.won) {
                location.assign(gameUrl.href);
                return;
            }
            html = outcome.html;
        }
    } catch (error) {
        control.error = error.message;
        control.debug = {
            capturedAt: new Date().toISOString(), pageUrl: location.href, userAgent: navigator.userAgent,
            error: { message: error.message, stack: error.stack },
            phase: control.phase, completed: control.completed, total: control.total,
            solver: control.solver ?? null, lastResult: control.result,
            puzzle, placements, expectedBoard: expected ?? null, lastMove, request: lastRequest,
        };
        control.report(`Stopped: ${error.message}`, 'error');
    } finally {
        control.running = false;
        onStatus?.(control.message, control);
    }
}
