import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

export function readPuzzles(path) {
    const puzzles = readFileSync(path, 'utf8').split(/\r?\n/).filter(line => line.trim()).map(line =>
        JSON.parse(line, (key, value, context) =>
            typeof value === 'number' && Number.isInteger(value) && !Number.isSafeInteger(value)
                ? BigInt(context.source) : value));
    assert(puzzles.length, 'No puzzles in input');
    return puzzles;
}

// Preserve 64-bit seed metadata when writing JSONL. Seeds are not sent to WASM.
export function stringify(value) {
    return JSON.stringify(value, (_, item) => typeof item === 'bigint' ? JSON.rawJSON(String(item)) : item);
}

export function verifySolution(puzzle, result) {
    assert(result.solved, stringify(result));
    assert.equal(result.placements.length, puzzle.pieces.length);
    const board = puzzle.board.map(row => [...row]);
    puzzle.pieces.forEach((piece, index) => {
        const [row, column] = result.placements[index];
        assert(Number.isInteger(row) && row >= 0 && row <= puzzle.rows - piece.length);
        assert(Number.isInteger(column) && column >= 0 && column <= puzzle.columns - piece[0].length);
        piece.forEach((cells, r) => cells.forEach((active, c) => {
            if (active) board[row + r][column + c] = (board[row + r][column + c] + puzzle.m - 1) % puzzle.m;
        }));
    });
    assert(board.every(row => row.every(value => value === 0)), 'Incorrect original-order placements');
}

export function puzzleHtml(puzzle) {
    const icons = puzzle.icons?.length ? puzzle.icons : ['hel', 'swo', 'cro', 'gob', 'sta'].slice(0, puzzle.m);
    const lines = [`LEVEL ${puzzle.level}; gX = ${puzzle.columns}; gY = ${puzzle.rows};`];
    puzzle.board.forEach((cells, row) => cells.forEach((value, column) => {
        lines.push(`imgLocStr[${column}][${row}] = "${icons[value]}";`);
    }));
    const cycle = [icons[0], ...icons.slice(1).reverse()];
    lines.push('<table>' + cycle.map((icon, i) =>
        `<img src="/${icon}_0.gif">` + (i === 0 ? '<br><b><small>GOAL</small></b>' : '')).join('') + '</table>');
    lines.push('ACTIVE SHAPE');
    puzzle.pieces.forEach((piece, index) => {
        if (index === 1) lines.push(puzzle.pieces.length === 2 ? 'NEXT SHAPE' : 'NEXT SHAPES');
        lines.push('<table border=0 cellpadding=0 cellspacing=0>');
        for (const cells of piece) lines.push('<tr>' + cells.map(active =>
            active ? '<td><img src="square.gif"></td>' : '<td></td>').join('') + '</tr>');
        lines.push('</table>');
    });
    lines.push('<a href="shapeshifter_instruct.phtml">Rules</a>');
    return lines.join('\n');
}

export async function openClient(browser, url, threads = null) {
    const page = await browser.newPage();
    // The copy page supplies isolation headers without starting a worker pool.
    const response = await page.goto(url);
    assert(response.ok(), `Page returned ${response.status()}: ${url}`);
    const info = await page.evaluate(async threads => {
        const { SolverClient } = await import('./search-client.js');
        window.events = [];
        window.client = new SolverClient({
            ...(threads === null ? {} : { threads }),
            onStatus: event => events.push(event),
        });
        return await client.init();
    }, threads);
    return { page, info };
}
