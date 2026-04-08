import init, { solve_puzzle } from './pkg/shapeshifter.js';

let wasmReady = false;

async function initWasm() {
    try {
        await init();
        wasmReady = true;
        document.getElementById('status').textContent = 'WASM loaded. Ready to solve.';
        document.getElementById('solve-btn').disabled = false;
    } catch (e) {
        document.getElementById('status').textContent = 'Failed to load WASM: ' + e.message;
        console.error(e);
    }
}

function renderBoard(puzzle, placements) {
    const container = document.getElementById('board-container');
    container.innerHTML = '';

    const rows = puzzle.rows;
    const cols = puzzle.columns;
    const m = puzzle.m;
    const board = puzzle.board.map(row => [...row]);

    // Build a map of which piece covers which cells
    const pieceMap = Array.from({ length: rows }, () => Array(cols).fill(-1));

    if (placements) {
        for (let pieceIdx = 0; pieceIdx < placements.length; pieceIdx++) {
            const [pr, pc] = placements[pieceIdx];
            const shape = puzzle.pieces[pieceIdx];
            for (let r = 0; r < shape.length; r++) {
                for (let c = 0; c < shape[r].length; c++) {
                    if (shape[r][c]) {
                        const br = pr + r;
                        const bc = pc + c;
                        if (br < rows && bc < cols) {
                            pieceMap[br][bc] = pieceIdx;
                            board[br][bc] = (board[br][bc] + 1) % m;
                        }
                    }
                }
            }
        }
    }

    const grid = document.createElement('div');
    grid.className = 'board-grid';
    grid.style.gridTemplateColumns = `repeat(${cols}, 1fr)`;

    // Color palette for pieces
    const colors = [
        '#e74c3c', '#3498db', '#2ecc71', '#f39c12', '#9b59b6',
        '#1abc9c', '#e67e22', '#e84393', '#00cec9', '#fdcb6e',
        '#6c5ce7', '#fab1a0', '#00b894', '#a29bfe', '#fd79a8',
        '#55a3e8', '#ff7675', '#74b9ff', '#a8e6cf', '#ffeaa7'
    ];

    for (let r = 0; r < rows; r++) {
        for (let c = 0; c < cols; c++) {
            const cell = document.createElement('div');
            cell.className = 'board-cell';
            cell.textContent = board[r][c];

            if (pieceMap[r][c] >= 0) {
                const colorIdx = pieceMap[r][c] % colors.length;
                cell.style.backgroundColor = colors[colorIdx];
                cell.style.color = '#fff';
                cell.style.fontWeight = 'bold';
                cell.title = `Piece ${pieceMap[r][c]} at (${r}, ${c})`;
            }

            // Highlight solved cells (value 0)
            if (placements && board[r][c] === 0) {
                cell.classList.add('solved-cell');
            }

            grid.appendChild(cell);
        }
    }

    container.appendChild(grid);
}

function renderInitialBoard(puzzle) {
    const container = document.getElementById('board-container');
    container.innerHTML = '';

    const rows = puzzle.rows;
    const cols = puzzle.columns;

    const grid = document.createElement('div');
    grid.className = 'board-grid';
    grid.style.gridTemplateColumns = `repeat(${cols}, 1fr)`;

    for (let r = 0; r < rows; r++) {
        for (let c = 0; c < cols; c++) {
            const cell = document.createElement('div');
            cell.className = 'board-cell';
            cell.textContent = puzzle.board[r][c];
            grid.appendChild(cell);
        }
    }

    container.appendChild(grid);
}

function renderPlacements(placements, pieces) {
    const list = document.getElementById('placements-list');
    list.innerHTML = '';

    for (let i = 0; i < placements.length; i++) {
        const [r, c] = placements[i];
        const li = document.createElement('li');

        // Describe piece shape
        const shape = pieces[i];
        const shapeRows = shape.length;
        const shapeCols = Math.max(...shape.map(r => r.length));
        const shapeStr = shape.map(row =>
            row.map(v => v ? '#' : '.').join('')
        ).join(' / ');

        li.innerHTML = `<strong>Piece ${i}</strong> (${shapeRows}x${shapeCols}): row=${r}, col=${c} <span class="shape-preview">${shapeStr}</span>`;
        list.appendChild(li);
    }
}

async function solvePuzzle() {
    if (!wasmReady) {
        alert('WASM module not loaded yet.');
        return;
    }

    const jsonInput = document.getElementById('puzzle-input').value.trim();
    if (!jsonInput) {
        alert('Please paste puzzle JSON.');
        return;
    }

    let puzzle;
    try {
        puzzle = JSON.parse(jsonInput);
    } catch (e) {
        document.getElementById('result-text').textContent = 'Invalid JSON: ' + e.message;
        return;
    }

    // Show initial board
    renderInitialBoard(puzzle);

    const solveBtn = document.getElementById('solve-btn');
    const resultText = document.getElementById('result-text');
    const statsText = document.getElementById('stats-text');

    solveBtn.disabled = true;
    resultText.textContent = 'Solving...';
    statsText.textContent = '';
    document.getElementById('placements-list').innerHTML = '';

    // Yield to UI before heavy computation
    await new Promise(resolve => setTimeout(resolve, 50));

    const startTime = performance.now();

    let resultJson;
    try {
        resultJson = solve_puzzle(jsonInput);
    } catch (e) {
        resultText.textContent = 'Error: ' + e.message;
        solveBtn.disabled = false;
        return;
    }

    const elapsed = performance.now() - startTime;

    let result;
    try {
        result = JSON.parse(resultJson);
    } catch (e) {
        resultText.textContent = 'Failed to parse solver result: ' + resultJson;
        solveBtn.disabled = false;
        return;
    }

    if (result.error) {
        resultText.textContent = 'Error: ' + result.error;
        solveBtn.disabled = false;
        return;
    }

    if (result.solved) {
        resultText.textContent = 'Solved!';
        resultText.className = 'result-solved';
        renderPlacements(result.placements, puzzle.pieces);
        renderBoard(puzzle, result.placements);
    } else {
        resultText.textContent = 'No solution found.';
        resultText.className = 'result-unsolved';
    }

    const nodesFormatted = result.nodes.toLocaleString();
    statsText.textContent = `Time: ${elapsed.toFixed(1)} ms | Nodes explored: ${nodesFormatted}`;

    solveBtn.disabled = false;
}

// Wire up the UI
document.addEventListener('DOMContentLoaded', () => {
    document.getElementById('solve-btn').addEventListener('click', solvePuzzle);
    initWasm();
});
