import { parseShapeshifterHtml } from './parser.js';
import { runMoves } from './move-script.js';

async function solveInFrame(puzzle, control) {
    control.report('Starting the solver in 3 seconds. Close DevTools for best WebAssembly performance.');
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
            if (error) reject(error);
            else resolve(result);
        }
        control.onStop = () => finish({ cancelled: true });
        frame.addEventListener('load', () => {
            if (!finished) frame.contentWindow.postMessage({ type: 'shapeshifter:solve', puzzle }, url.origin, [port2]);
        }, { once: true });
        port1.onmessage = ({ data }) => {
            if (data.type === 'result') finish(data.result);
            else if (data.type === 'error') finish(null, new Error(data.message));
            else if (data.type === 'status') {
                const status = data.status;
                if (status.type === 'ready') {
                    clearTimeout(timer);
                    timer = setTimeout(() => finish(null, new Error('Hosted solver stopped responding.')), 300000);
                    control.report(`Solver ready: ${status.workers} search workers.`);
                    if (!status.threaded) control.report('Shared memory is unavailable here; this run uses one worker.', 'warn');
                } else if (status.type === 'preparing') control.report('Preparing puzzle…');
                else if (status.type === 'searching') control.report('Searching (2 minute budget)…');
            }
        };
        document.body.append(frame);
    });
}

export function run() {
    let panel, text, button;
    return runMoves(null, null, parseShapeshifterHtml, 1000, solveInFrame, (message, control) => {
        if (!panel) {
            document.getElementById('shapeshifter-status')?.remove();
            panel = document.createElement('aside');
            panel.id = 'shapeshifter-status';
            panel.style.cssText = 'position:fixed;bottom:16px;right:16px;z-index:2147483647;'
                + 'max-width:420px;padding:16px;background:#172033;color:#fff;border-radius:8px;'
                + 'box-shadow:0 2px 12px #0006;font:14px/1.5 system-ui;text-align:left;';
            text = document.createElement('div');
            text.setAttribute('role', 'status');
            button = document.createElement('button');
            button.style.cssText = 'margin-top:8px;padding:4px 12px;cursor:pointer;font:inherit;';
            button.onclick = () => {
                if (control.running) {
                    control.stop();
                    control.report('Stopping…');
                } else panel.remove();
            };
            panel.append(text, button);
            document.body.append(panel);
        }
        text.textContent = message;
        button.textContent = control.running ? 'Stop' : 'Dismiss';
        button.disabled = control.running && control.stopped;
    });
}
