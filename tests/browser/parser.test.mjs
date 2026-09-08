import assert from 'node:assert/strict';
import test from 'node:test';
import { puzzleHtml } from '../../scripts/browser-tools.mjs';
import { parseShapeshifterHtml } from '../../web/parser.js';

const puzzle = { level: 100, rows: 3, columns: 3, m: 5,
    board: [[0, 1, 2], [3, 4, 0], [1, 2, 3]], pieces: [[[true]]] };
const expected = { ...puzzle, icons: ['hel', 'swo', 'cro', 'gob', 'sta'] };
const html = puzzleHtml(puzzle);
const hint = '<table><tr><td>Sinsi says: match the symbol above the word <b>GOAL</b> to win.</td></tr></table>';

test('hint text mentioning GOAL does not change the board or symbol cycle', () => {
    assert.deepEqual(parseShapeshifterHtml(html), expected);
    assert.deepEqual(parseShapeshifterHtml(hint + html), expected);
});

test('the labelled goal determines the cycle with a wrap and nested tables', () => {
    const cycle = `<table><tr><td><table><tr>
        <td><img src='/cro_0.gif'></td><td><img src='/arrow.gif'></td>
        <td><img src='/swo_0.gif'></td><td><img src='/arrow.gif'></td>
        <td><img src='/hel_0.gif'><br />\n<b><small> GOAL </small></b></td>
        <td><img src='/arrow.gif'></td><td><img src='/sta_0.gif'></td>
        <td><img src='/arrow.gif'></td><td><img src='/gob_0.gif'></td>
        <td><img src='/arrow.gif'></td><td><img src='/cro_0.gif'></td>
        </tr></table></td></tr></table>`;
    assert.deepEqual(parseShapeshifterHtml(hint + html.replace(/<table>.*<\/table>/, cycle)), expected);
});

test('missing or invalid goal cycles fail instead of guessing a symbol order', () => {
    for (const invalid of [
        html.replace(/<table>.*<\/table>/, ''),
        html.replace('<small>GOAL</small>', '<small>Target</small>'),
        html.replace('/sta_0.gif', '/hel_0.gif'),
        html.replace('/hel_0.gif', '/arrow_0.gif'),
    ]) {
        assert.throws(() => parseShapeshifterHtml(hint + invalid), /Could not read the symbol cycle/);
    }
});
