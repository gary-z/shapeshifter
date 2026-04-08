import init, { solve_puzzle } from './pkg/shapeshifter.js';
import { parseShapeshifterHtml } from './parser.js';

const ASSETS_DIR = 'web/assets';

let wasmReady = false;

async function initWasm() {
    try {
        await init();
        wasmReady = true;
        document.getElementById('status').textContent = 'Ready to solve.';
        document.getElementById('solve-btn').disabled = false;
    } catch (e) {
        document.getElementById('status').textContent = 'Failed to load WASM: ' + e.message;
        console.error(e);
    }
}

function iconSrc(icons, val) {
    if (icons.length > 0 && val < icons.length && icons[val]) {
        return `${ASSETS_DIR}/${icons[val]}_0.gif`;
    }
    return null;
}

function iconSrcHighlight(icons, val) {
    if (icons.length > 0 && val < icons.length && icons[val]) {
        return `${ASSETS_DIR}/${icons[val]}_1.gif`;
    }
    return null;
}

function renderBoardHtml(board, h, w, icons, pieceMask, clickPos) {
    let s = `<div class="board" style="grid-template-columns: repeat(${w}, 50px)">\n`;
    for (let r = 0; r < h; r++) {
        for (let c = 0; c < w; c++) {
            const val = board[r][c];
            const isPiece = pieceMask ? pieceMask[r * w + c] : false;
            const isClick = clickPos ? (r === clickPos[0] && c === clickPos[1]) : false;
            const cls = isClick ? 'cell highlight click-here'
                : isPiece ? 'cell highlight' : 'cell';
            const src = isPiece ? iconSrcHighlight(icons, val) : iconSrc(icons, val);
            if (src) {
                s += `<div class="${cls}"><img src="${src}"></div>\n`;
            } else {
                // Fallback: colored number cell
                const colors = ['#2ecc71', '#e74c3c', '#3498db', '#f39c12', '#9b59b6'];
                const bg = colors[val % colors.length];
                const border = isPiece ? '3px solid #ff0' : isClick ? '3px solid #2ecc40' : 'none';
                s += `<div class="${cls}" style="background:${bg};color:#fff;display:flex;align-items:center;justify-content:center;font-weight:bold;font-size:18px;width:50px;height:50px;border:${border}">${val}</div>\n`;
            }
        }
    }
    s += '</div>\n';
    return s;
}

function applyPiece(board, piece, row, col, m) {
    const h = board.length, w = board[0].length;
    for (let r = 0; r < piece.length; r++) {
        for (let c = 0; c < piece[r].length; c++) {
            if (piece[r][c]) {
                const br = row + r, bc = col + c;
                if (br < h && bc < w) {
                    board[br][bc] = (board[br][bc] + m - 1) % m;
                }
            }
        }
    }
}

function buildPieceMask(piece, row, col, h, w) {
    const mask = Array(h * w).fill(false);
    for (let r = 0; r < piece.length; r++) {
        for (let c = 0; c < piece[r].length; c++) {
            if (piece[r][c]) {
                const br = row + r, bc = col + c;
                if (br < h && bc < w) mask[br * w + bc] = true;
            }
        }
    }
    return mask;
}

function generateSolutionGuide(puzzle, placements) {
    const { m, rows: h, columns: w, pieces, icons } = puzzle;
    const board = puzzle.board.map(r => [...r]);

    let html = `<h2>Level ${puzzle.level} Solution</h2>
<p class="info">${h}&times;${w} board, M=${m}, ${pieces.length} pieces</p>\n`;

    for (let i = 0; i < placements.length; i++) {
        const [row, col] = placements[i];
        const piece = pieces[i];
        const mask = buildPieceMask(piece, row, col, h, w);

        // Piece shape preview
        const shapeStr = piece.map(r => r.map(v => v ? '\u2588' : '\u00B7').join('')).join('\n');

        html += `<div class="step">
<h3>Step ${i + 1}: Place piece at row ${row}, col ${col}</h3>
<pre class="shape-preview">${shapeStr}</pre>\n`;
        html += renderBoardHtml(board, h, w, icons, mask, [row, col]);

        applyPiece(board, piece, row, col, m);
        html += '</div>\n';
    }

    // Final board
    const allZero = board.every(r => r.every(v => v === 0));
    html += '<div class="step"><h3>Result</h3>\n';
    html += renderBoardHtml(board, h, w, icons, null, null);
    if (allZero) {
        html += '<div class="solved">SOLVED!</div>\n';
    } else {
        html += '<div class="solved" style="color:#e74c3c">NOT SOLVED</div>\n';
    }
    html += '</div>\n';

    return html;
}

async function solvePuzzle() {
    if (!wasmReady) { alert('WASM not loaded yet.'); return; }

    const input = document.getElementById('puzzle-input').value.trim();
    if (!input) { alert('Paste the Shapeshifter page HTML.'); return; }

    const resultDiv = document.getElementById('results-content');
    const statsText = document.getElementById('stats-text');

    // Parse HTML to JSON
    let puzzle;
    try {
        puzzle = parseShapeshifterHtml(input);
    } catch (e) {
        resultDiv.innerHTML = `<p style="color:#e74c3c">Parse error: ${e.message}</p>`;
        return;
    }

    const puzzleJson = JSON.stringify(puzzle);
    resultDiv.innerHTML = `<p>Parsed level ${puzzle.level}: ${puzzle.rows}&times;${puzzle.columns}, M=${puzzle.m}, ${puzzle.pieces.length} pieces</p><p>Solving...</p>`;
    statsText.textContent = '';

    document.getElementById('solve-btn').disabled = true;
    await new Promise(resolve => setTimeout(resolve, 50));

    const startTime = performance.now();

    let solverResult;
    try {
        const resultJson = solve_puzzle(puzzleJson);
        solverResult = JSON.parse(resultJson);
    } catch (e) {
        resultDiv.innerHTML = `<p style="color:#e74c3c">Solver error: ${e.message}</p>`;
        document.getElementById('solve-btn').disabled = false;
        return;
    }

    const elapsed = performance.now() - startTime;

    if (solverResult.error) {
        resultDiv.innerHTML = `<p style="color:#e74c3c">Error: ${solverResult.error}</p>`;
    } else if (solverResult.solved) {
        resultDiv.innerHTML = generateSolutionGuide(puzzle, solverResult.placements);
    } else {
        resultDiv.innerHTML = '<p style="color:#e74c3c">No solution found.</p>';
    }

    statsText.textContent = `Time: ${elapsed.toFixed(0)} ms | Nodes: ${solverResult.nodes.toLocaleString()}`;
    document.getElementById('solve-btn').disabled = false;
}

document.addEventListener('DOMContentLoaded', () => {
    document.getElementById('solve-btn').addEventListener('click', solvePuzzle);
    initWasm();
});
