import init, { solve_puzzle } from './pkg/shapeshifter.js';
import { parseShapeshifterHtml } from './parser.js';

const ASSETS_DIR = 'web/assets';
const DEFAULT_ICONS = ['swo', 'hel'];

let wasmReady = false;

async function initWasm() {
    try {
        await init();
        wasmReady = true;
        document.getElementById('solve-btn').disabled = false;
    } catch (e) {
        document.getElementById('status').textContent = 'Failed to load WASM: ' + e.message;
        console.error(e);
    }
}

async function solvePuzzle() {
    if (!wasmReady) { alert('WASM not loaded yet.'); return; }

    const input = document.getElementById('puzzle-input').value.trim();
    if (!input) { alert('Paste the Shapeshifter page HTML.'); return; }

    const resultDiv = document.getElementById('results-content');

    let puzzle;
    try {
        puzzle = parseShapeshifterHtml(input);
    } catch (e) {
        resultDiv.innerHTML = `<p style="color:#e74c3c">Parse error: ${e.message}</p>`;
        return;
    }

    const puzzleJson = JSON.stringify(puzzle);
    resultDiv.innerHTML = '<p style="color:#aaa">Solving...</p>';

    document.getElementById('solve-btn').disabled = true;
    await new Promise(resolve => setTimeout(resolve, 50));

    let solverResult;
    try {
        const resultJson = solve_puzzle(puzzleJson);
        solverResult = JSON.parse(resultJson);
    } catch (e) {
        resultDiv.innerHTML = `<p style="color:#e74c3c">Solver error: ${e.message}</p>`;
        document.getElementById('solve-btn').disabled = false;
        return;
    }

    if (solverResult.error) {
        resultDiv.innerHTML = `<p style="color:#e74c3c">Error: ${solverResult.error}</p>`;
    } else if (solverResult.solved) {
        boardShowSolution(resultDiv, puzzle, solverResult.placements, ASSETS_DIR);
    } else {
        resultDiv.innerHTML = '<p style="color:#e74c3c">No solution found.</p>';
    }

    document.getElementById('solve-btn').disabled = false;
}

function showDefaultBoard() {
    const board = Array.from({length: 6}, () => Array(6).fill(0));
    const container = document.getElementById('results-content');
    boardRender(container, board, 6, 6, DEFAULT_ICONS, ASSETS_DIR, null, null);
}

document.addEventListener('DOMContentLoaded', () => {
    document.getElementById('solve-btn').addEventListener('click', solvePuzzle);
    showDefaultBoard();
    initWasm();
});
