import { parseShapeshifterHtml } from './parser.js';
import { runMoves } from './move-script.js';

const SEARCH_BUDGET_MS = 120000;

async function solveInFrame(puzzle, control) {
    control.solver = null;
    control.report('');
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
                }
            }
        };
        document.body.append(frame);
    });
}

export function run() {
    let panel, text, badge, button;
    return runMoves(null, null, parseShapeshifterHtml, 1000, solveInFrame, (message, control) => {
        if (!control.running && !control.error) {
            panel?.remove();
            return;
        }
        if (!panel) {
            document.getElementById('shapeshifter-status')?.remove();
            panel = document.createElement('aside');
            panel.id = 'shapeshifter-status';
            panel.style.cssText = 'position:fixed;bottom:16px;right:16px;z-index:2147483647;'
                + 'max-width:420px;padding:16px;background:#172033;color:#fff;border-radius:8px;'
                + 'box-shadow:0 2px 12px #0006;font:14px/1.5 system-ui;text-align:left;';
            text = document.createElement('div');
            text.setAttribute('role', 'status');
            badge = document.createElement('span');
            badge.className = 'solver-features';
            badge.style.cssText = 'margin-right:12px;font-size:12px;';
            badge.title = 'Loaded solver features. Browser JIT optimization cannot be verified here; close DevTools for best performance.';
            button = document.createElement('button');
            button.style.cssText = 'margin-top:8px;padding:4px 12px;cursor:pointer;font:inherit;';
            button.onclick = () => {
                if (control.running) control.stop();
                else panel.remove();
            };
            panel.append(text, badge, button);
            document.body.append(panel);
        }
        text.textContent = message;
        text.hidden = !message;
        const info = control.solver;
        badge.hidden = !info?.workers || info.type === 'finished';
        if (info?.workers) {
            // The threaded package is built with SIMD; the fallback package uses scalar instructions.
            badge.textContent = `${info.threaded ? 'SIMD' : 'Scalar'} · ${info.workers} worker${info.workers === 1 ? '' : 's'}`;
            badge.style.color = info.threaded ? '#9ae6b4' : '#f6d58b';
        }
        panel.style.padding = message ? '16px' : '8px';
        button.style.marginTop = message ? '8px' : '0';
        button.textContent = control.running ? 'Stop' : 'Dismiss';
        button.disabled = control.running && control.stopped;
    });
}
