// Shared board rendering and step-through component.
// Used by both the WASM web app (solver.js) and standalone solution.html files.

function boardIconSrc(assetsDir, icons, val, highlight) {
    const suffix = highlight ? '_1' : '_0';
    if (icons && icons.length > 0 && val < icons.length && icons[val]) {
        return `${assetsDir}/${icons[val]}${suffix}.gif`;
    }
    return null;
}

function boardRender(container, board, h, w, icons, assetsDir, pieceMask, clickPos) {
    container.innerHTML = '';
    const grid = document.createElement('div');
    grid.className = 'board';
    grid.style.gridTemplateColumns = `repeat(${w}, 50px)`;

    for (let r = 0; r < h; r++) {
        for (let c = 0; c < w; c++) {
            const val = board[r][c];
            const isPiece = pieceMask ? pieceMask[r * w + c] : false;
            const isClick = clickPos ? (r === clickPos[0] && c === clickPos[1]) : false;

            const cell = document.createElement('div');
            cell.className = isClick ? 'cell click-here' : 'cell';

            const src = boardIconSrc(assetsDir, icons, val, isPiece);
            if (src) {
                const img = document.createElement('img');
                img.src = src;
                cell.appendChild(img);
            } else {
                const colors = ['#2ecc71', '#e74c3c', '#3498db', '#f39c12', '#9b59b6'];
                cell.style.cssText = `background:${colors[val % colors.length]};color:#fff;display:flex;align-items:center;justify-content:center;font-weight:bold;font-size:18px;width:50px;height:50px`;
                cell.textContent = val;
            }
            grid.appendChild(cell);
        }
    }
    container.appendChild(grid);
}

function boardApplyPiece(board, piece, row, col, m) {
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

function boardBuildPieceMask(piece, row, col, h, w) {
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

function boardBuildSteps(puzzle, placements) {
    const { m, rows: h, columns: w, pieces } = puzzle;
    const steps = [];
    const board = puzzle.board.map(r => [...r]);

    steps.push({
        board: board.map(r => [...r]),
        label: 'Starting board',
        pieceMask: null,
        clickPos: null,
    });

    for (let i = 0; i < placements.length; i++) {
        const [row, col] = placements[i];
        const piece = pieces[i];
        const mask = boardBuildPieceMask(piece, row, col, h, w);

        steps.push({
            board: board.map(r => [...r]),
            label: `Step ${i + 1}/${placements.length}: place at (${row}, ${col})`,
            pieceMask: mask,
            clickPos: [row, col],
        });

        boardApplyPiece(board, piece, row, col, m);
    }

    const allZero = board.every(r => r.every(v => v === 0));
    steps.push({
        board: board.map(r => [...r]),
        label: allZero ? 'Solved!' : 'Result (not solved)',
        pieceMask: null,
        clickPos: null,
        solved: allZero,
    });

    return steps;
}

function boardShowSolution(container, puzzle, placements, assetsDir) {
    const icons = puzzle.icons && puzzle.icons.length > 0 && puzzle.icons[0]
        ? puzzle.icons : ['swo', 'hel'].slice(0, puzzle.m);
    const { rows: h, columns: w } = puzzle;
    const steps = boardBuildSteps(puzzle, placements);
    let currentStep = steps.length > 2 ? 1 : 0;

    container.innerHTML = '';

    const nav = document.createElement('div');
    nav.className = 'step-nav';
    const prevBtn = document.createElement('button');
    prevBtn.textContent = '\u25C0 Prev';
    const nextBtn = document.createElement('button');
    nextBtn.textContent = 'Next \u25B6';
    const label = document.createElement('div');
    label.className = 'step-label';
    nav.appendChild(prevBtn);
    nav.appendChild(label);
    nav.appendChild(nextBtn);
    container.appendChild(nav);

    const boardDiv = document.createElement('div');
    boardDiv.style.textAlign = 'center';
    container.appendChild(boardDiv);

    const banner = document.createElement('div');
    banner.className = 'solved';
    banner.style.display = 'none';
    container.appendChild(banner);

    function render() {
        const step = steps[currentStep];
        label.textContent = step.label;
        prevBtn.disabled = currentStep === 0;
        nextBtn.disabled = currentStep === steps.length - 1;

        boardRender(boardDiv, step.board, h, w, icons, assetsDir, step.pieceMask, step.clickPos);

        if (step.solved !== undefined) {
            banner.style.display = 'block';
            banner.textContent = step.solved ? 'SOLVED!' : 'NOT SOLVED';
            banner.style.color = step.solved ? '#2ecc71' : '#e74c3c';
        } else {
            banner.style.display = 'none';
        }
    }

    prevBtn.addEventListener('click', () => {
        if (currentStep > 0) { currentStep--; render(); }
    });
    nextBtn.addEventListener('click', () => {
        if (currentStep < steps.length - 1) { currentStep++; render(); }
    });

    document.addEventListener('keydown', (e) => {
        if (e.key === 'ArrowLeft' || e.key === 'a') {
            if (currentStep > 0) { currentStep--; render(); }
        } else if (e.key === 'ArrowRight' || e.key === 'd') {
            if (currentStep < steps.length - 1) { currentStep++; render(); }
        }
    });

    render();
}
