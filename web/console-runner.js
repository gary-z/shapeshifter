import { parseShapeshifterHtml } from './parser.js';
import { runMoves } from './move-script.js';

async function solveInFrame(puzzle, control) {
    console.log('Starting the solver in 3 seconds. Close DevTools for best WebAssembly performance.');
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
                    console.log(`Solver ready: ${status.workers} search workers.`);
                    if (!status.threaded) console.warn('Shared memory is unavailable here; this run uses one worker.');
                } else if (status.type === 'preparing') console.log('Preparing puzzle…');
                else if (status.type === 'searching') console.log('Searching (2 minute budget)…');
            }
        };
        document.body.append(frame);
    });
}

export function run() {
    return runMoves(null, null, parseShapeshifterHtml, 1000, solveInFrame);
}
