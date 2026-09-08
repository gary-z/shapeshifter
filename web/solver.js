import { SolverClient } from './search-client.js';
import { parseShapeshifterHtml } from './parser.js';

const ASSETS_DIR = 'assets';
const DEFAULT_ICONS = ['swo', 'hel'];
const status = document.getElementById('status');
const solveButton = document.getElementById('solve-btn');
const cancelButton = document.getElementById('cancel-btn');
const input = document.getElementById('puzzle-input');
const results = document.getElementById('results-content');
let searchTimer;
let parsedPuzzle = null;
let loading = true;

const solver = new SolverClient({ onStatus(update) {
    if (update.type === 'loading') status.textContent = 'Loading solver…';
    if (update.type === 'ready') status.textContent = update.threaded
        ? `Ready · ${update.workers} search workers · 2 minute search budget`
        : 'Ready · 1 search worker · 2 minute search budget (parallel mode unavailable)';
    if (update.type === 'preparing') status.textContent = 'Preparing puzzle…';
    if (update.type === 'searching') {
        const started = performance.now();
        const tick = () => {
            const seconds = Math.floor((performance.now() - started) / 1000);
            status.textContent = `Searching · ${seconds} / 120 seconds · ${update.workers} search worker${update.workers === 1 ? '' : 's'}`;
        };
        tick();
        searchTimer = setInterval(tick, 250);
    }
} });

function message(text) {
    results.replaceChildren();
    const paragraph = document.createElement('p');
    paragraph.textContent = text;
    results.append(paragraph);
}

function updateSolveButton() {
    solveButton.hidden = !parsedPuzzle;
    solveButton.disabled = !parsedPuzzle || loading || input.disabled;
}

function previewPuzzle() {
    parsedPuzzle = null;
    input.setAttribute('aria-invalid', 'false');
    try {
        const html = input.value.trim();
        if (html) {
            const puzzle = parseShapeshifterHtml(html);
            boardRender(results, puzzle.board, puzzle.rows, puzzle.columns,
                puzzle.icons, ASSETS_DIR, null, null);
            parsedPuzzle = puzzle;
        } else {
            boardRender(results, Array.from({ length: 6 }, () => Array(6).fill(0)),
                6, 6, DEFAULT_ICONS, ASSETS_DIR, null, null);
        }
    } catch (error) {
        input.setAttribute('aria-invalid', 'true');
        message(`Parse error: ${error.message}`);
    }
    updateSolveButton();
}

async function solvePuzzle() {
    const puzzle = parsedPuzzle;
    if (!puzzle || loading || input.disabled) return;

    input.disabled = true;
    updateSolveButton();
    cancelButton.hidden = false;
    cancelButton.disabled = false;
    status.textContent = 'Preparing puzzle…';
    try {
        const result = await solver.solve(puzzle);
        if (result.cancelled) message('Search cancelled.');
        else if (result.solved) boardShowSolution(results, puzzle, result.placements, ASSETS_DIR);
        else if (result.timed_out) message('No solution found within 2 minutes. You can try again.');
        else message('No solution found.');
        status.textContent = result.search_ms === undefined ? 'Search cancelled.'
            : `${(result.search_ms / 1000).toFixed(2)}s search · ${(result.preparation_ms / 1000).toFixed(2)}s preparation`;
    } catch (error) {
        message(`Solver error: ${error.message}`);
        status.textContent = 'Solver stopped. Try again.';
    } finally {
        clearInterval(searchTimer);
        input.disabled = false;
        updateSolveButton();
        cancelButton.hidden = true;
    }
}

input.addEventListener('input', previewPuzzle);
solveButton.addEventListener('click', solvePuzzle);
cancelButton.addEventListener('click', () => {
    cancelButton.disabled = true;
    clearInterval(searchTimer);
    status.textContent = 'Cancelling…';
    solver.cancel();
});
previewPuzzle();
solver.init().catch(error => {
    status.textContent = `Failed to load solver: ${error.message}`;
}).finally(() => {
    loading = false;
    updateSolveButton();
});
