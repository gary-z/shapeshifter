import { parseShapeshifterHtml } from './parser.js';
import { runMoves } from './move-script.js';
import { copyOnClick } from './copy-button.js';

const SEARCH_BUDGET_MS = 120000;

async function solveInFrame(puzzle, control) {
    control.update({ solver: null, phase: 'loading', searchDeadline: null });
    await new Promise(resolve => setTimeout(resolve, 3000));
    if (control.stopped) return { cancelled: true };

    return new Promise((resolve, reject) => {
        const url = new URL('./console-solver.html', import.meta.url);
        const frame = document.createElement('iframe');
        frame.hidden = true;
        frame.title = 'Shapeshifter solver';
        frame.allow = 'cross-origin-isolated';
        frame.src = url.href;
        const { port1, port2 } = new MessageChannel();
        let finished = false;
        let timer = setTimeout(() => finish(null, new Error('Hosted solver failed to load.')), 35000);
        function finish(result, error) {
            if (finished) return;
            finished = true;
            clearTimeout(timer);
            control.onStop = null;
            port1.close();
            port2.close();
            frame.remove();
            if (control.solver) control.solver.type = 'finished';
            if (!error) control.phase = 'checking';
            control.searchDeadline = null;
            control.report('');
            if (error) reject(error);
            else resolve(result);
        }
        control.onStop = () => finish({ cancelled: true });
        frame.addEventListener('load', () => {
            if (!finished) frame.contentWindow.postMessage({ type: 'shapeshifter:solve', puzzle, budgetMs: SEARCH_BUDGET_MS }, url.origin, [port2]);
        }, { once: true });
        port1.onmessage = ({ data }) => {
            if (data.type === 'result') finish(data.result);
            else if (data.type === 'error') finish(null, new Error(data.message));
            else if (data.type === 'status') {
                const status = data.status;
                control.solver = status;
                if (status.type === 'ready') {
                    clearTimeout(timer);
                    timer = setTimeout(() => finish(null, new Error('Hosted solver stopped responding.')), 300000);
                    control.report('');
                } else if (status.type === 'fallback') {
                    control.report('Parallel solver failed to start. Using one worker.');
                } else if (status.type === 'preparing') {
                    control.update({ phase: 'preparing' });
                } else if (status.type === 'searching') {
                    control.update({ phase: 'searching', searchDeadline: performance.now() + SEARCH_BUDGET_MS });
                }
            }
        };
        document.body.append(frame);
    });
}

export function run() {
    let panel, text, progress, badge, button, copy, ticker;
    return runMoves(null, null, parseShapeshifterHtml, 1000, solveInFrame, (message, control) => {
        clearInterval(ticker);
        if (!control.running && !control.error) {
            panel?.remove();
            return;
        }
        if (!panel) {
            document.getElementById('shapeshifter-status')?.remove();
            panel = document.createElement('aside');
            panel.id = 'shapeshifter-status';
            panel.style.cssText = 'position:fixed;bottom:16px;right:16px;z-index:2147483647;'
                + 'min-width:240px;max-width:420px;padding:16px;background:#172033;color:#fff;border-radius:8px;'
                + 'box-shadow:0 2px 12px #0006;font:14px/1.5 system-ui;text-align:left;';
            text = document.createElement('div');
            text.className = 'solver-message';
            text.setAttribute('role', 'status');
            progress = document.createElement('div');
            progress.className = 'solver-progress';
            progress.style.cssText = 'margin-bottom:8px;font-variant-numeric:tabular-nums;';
            badge = document.createElement('span');
            badge.className = 'solver-features';
            badge.hidden = true;
            badge.style.cssText = 'margin-right:12px;font-size:12px;';
            badge.title = 'Features used by the latest search. Browser JIT optimization cannot be verified here; close DevTools for best performance.';
            button = document.createElement('button');
            button.style.cssText = 'padding:4px 12px;cursor:pointer;font:inherit;';
            button.onclick = () => {
                if (control.running) control.stop();
                else panel.remove();
            };
            panel.append(text, progress, badge, button);
            document.body.append(panel);
        }
        text.textContent = message;
        text.hidden = !message;
        const searching = control.phase === 'searching';
        progress.hidden = !!control.error;
        progress.setAttribute('role', searching ? 'timer' : 'status');
        function showProgress() {
            if (control.stopped) progress.textContent = 'Stopping…';
            else if (searching) {
                const seconds = Math.max(0, Math.ceil((control.searchDeadline - performance.now()) / 1000));
                progress.textContent = `Searching · ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')} remaining`;
            } else if (control.phase === 'placing') {
                progress.textContent = `Placing pieces · ${control.completed} / ${control.total}`;
            } else {
                progress.textContent = { reading: 'Reading game…', loading: 'Loading solver…',
                    preparing: 'Preparing puzzle…', checking: 'Checking board…', restarting: 'Starting a new puzzle…' }[control.phase];
            }
        }
        showProgress();
        if (searching && control.running && !control.stopped && !control.error) ticker = setInterval(showProgress, 250);
        const info = control.solver;
        if (info?.workers) {
            badge.hidden = false;
            // The threaded package is built with SIMD; the fallback package uses scalar instructions.
            badge.textContent = `${info.threaded ? 'SIMD' : 'Scalar'} · ${info.workers} worker${info.workers === 1 ? '' : 's'}`;
            badge.style.color = info.threaded ? '#9ae6b4' : '#f6d58b';
        }
        button.style.marginTop = message && control.error ? '8px' : '0';
        button.textContent = control.running ? 'Stop' : 'Dismiss';
        button.disabled = control.running && control.stopped;
        if (control.error && !copy) {
            copy = document.createElement('button');
            copy.className = 'solver-copy-debug';
            copy.textContent = 'Copy debug info';
            copy.title = 'Copy error details and the last game response HTML.';
            copy.style.cssText = 'margin:8px 0 0 8px;padding:4px 12px;cursor:pointer;font:inherit;';
            copyOnClick(copy, JSON.stringify({ runnerUrl: import.meta.url, ...control.debug }, null, 2));
            panel.append(copy);
        }
    });
}
