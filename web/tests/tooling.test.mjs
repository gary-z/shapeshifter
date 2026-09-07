import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { serve } from '../serve.mjs';
import { normalizePackages } from '../normalize-packages.mjs';
import { readPuzzles, stringify, verifySolution } from '../browser-tools.mjs';

async function temporary(t) {
    const path = await mkdtemp(join(tmpdir(), 'shapeshifter-tools-'));
    t.after(() => rm(path, { recursive: true, force: true }));
    return path;
}

test('server supplies WASM MIME and isolation headers, with fallback and failure modes', async t => {
    const root = await temporary(t);
    await writeFile(join(root, 'index.html'), '<html>fixture</html>');
    await writeFile(join(root, 'module.wasm'), Buffer.from([0, 97, 115, 109]));
    for (const isolated of [true, false]) {
        const server = await serve({ root, isolated, failThreaded: true });
        try {
            const response = await fetch(server.url + '/module.wasm?cache=1');
            assert.equal(response.status, 200);
            assert.equal(response.headers.get('content-type'), 'application/wasm');
            assert.equal(response.headers.get('cross-origin-opener-policy'), isolated ? 'same-origin' : null);
            assert.equal(response.headers.get('cross-origin-embedder-policy'), isolated ? 'require-corp' : null);
            assert.deepEqual(Buffer.from(await response.arrayBuffer()), Buffer.from([0, 97, 115, 109]));
            const head = await fetch(server.url + '/', { method: 'HEAD' });
            assert.equal(head.status, 200);
            assert.equal(await head.text(), '');
            assert.equal((await fetch(server.url + '/web/pkg-threaded/module.js')).status, 503);
            assert.equal((await fetch(server.url + '/missing')).status, 404);
            assert.equal((await fetch(server.url + '/%ff')).status, 400);
            assert.equal((await fetch(server.url + '/', { method: 'POST' })).status, 405);
        } finally { await server.close(); }
    }
});

test('server confines files to its root, including symlinks', async t => {
    const directory = await temporary(t);
    const root = join(directory, 'site');
    await mkdir(root);
    await writeFile(join(directory, 'outside'), 'outside');
    await symlink(join(directory, 'outside'), join(root, 'link'));
    const server = await serve({ root });
    try {
        assert.equal((await fetch(server.url + '/..%2foutside')).status, 403);
        assert.equal((await fetch(server.url + '/link')).status, 403);
    } finally { await server.close(); }
});

test('normalization changes CRLF in nested JavaScript without touching WASM', async t => {
    const root = await temporary(t);
    await mkdir(join(root, 'nested'));
    await writeFile(join(root, 'nested/helper.js'), 'a\r\nb\rc\n');
    await writeFile(join(root, 'module.wasm'), Buffer.from([13, 10, 0, 255]));
    normalizePackages([root]);
    normalizePackages([root]);
    assert.equal(await readFile(join(root, 'nested/helper.js'), 'utf8'), 'a\nb\rc\n');
    assert.deepEqual(await readFile(join(root, 'module.wasm')), Buffer.from([13, 10, 0, 255]));
});

test('JSONL preserves full-width seed integers', async t => {
    const root = await temporary(t);
    const path = join(root, 'puzzles.jsonl');
    await writeFile(path, '\n{"seed":18446744073709551615,"level":100}\n{"level":1}\n');
    const puzzles = readPuzzles(path);
    assert.equal(puzzles[0].seed, 18446744073709551615n);
    assert.equal(stringify(puzzles[0]), '{"seed":18446744073709551615,"level":100}');
    assert.equal(puzzles[1].seed, undefined);
});

test('solution replay handles modular subtraction and rejects invalid placements', () => {
    const puzzle = { rows: 1, columns: 2, m: 3, board: [[2, 1]], pieces: [[[true, true]], [[true]]] };
    const result = { solved: true, placements: [[0, 0], [0, 0]] };
    verifySolution(puzzle, result);
    assert.throws(() => verifySolution(puzzle, { ...result, placements: [[0, 0], [0, 1]] }));
    assert.throws(() => verifySolution(puzzle, { ...result, placements: [[0, 0], [1, 0]] }));
    assert.throws(() => verifySolution(puzzle, { ...result, placements: [[0, 0], [0.5, 0]] }));
});
