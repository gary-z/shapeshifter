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

const largePiece = Array.from({ length: 5 }, () => Array(5).fill(true));
for (const pieces of [[largePiece], [[[true]], largePiece], [[[true]], largePiece, [[true]]]]) {
    test(`reads ${pieces.length} remaining shape${pieces.length === 1 ? '' : 's'} with full-size game markup`, () => {
        const input = { ...puzzle, rows: 5, columns: 5,
            board: Array.from({ length: 5 }, () => Array(5).fill(0)), pieces };
        const page = puzzleHtml(input).replaceAll('<img src="square.gif">',
            '<img src="//images.neopets.com/medieval/shapeshifter/square.gif" width=10 height=10 border=0>');
        assert(page.length - page.indexOf('ACTIVE SHAPE') > 2000);
        assert.deepEqual(parseShapeshifterHtml(page), { ...input, icons: expected.icons });
    });
}

test('shape parsing stops at the game instructions', () => {
    const footer = '<table border=0 cellpadding=0 cellspacing=0><tr><td><img src="square.gif"></td></tr></table>';
    assert.deepEqual(parseShapeshifterHtml(html + footer), expected);
});

test('missing level and incomplete piece sections fail instead of guessing', () => {
    assert.throws(() => parseShapeshifterHtml(html.replace('LEVEL 100', '')),
        /Could not find the level/);
    assert.throws(() => parseShapeshifterHtml(html.slice(0, html.indexOf('<a href="shapeshifter_instruct'))),
        /Could not find the end of the piece list/);
    assert.throws(() => parseShapeshifterHtml(html.replace('ACTIVE SHAPE', '')),
        /Could not find any pieces/);
});
