import { createServer } from 'node:http';
import { readFile, realpath, stat } from 'node:fs/promises';
import { extname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const SITE_ROOT = fileURLToPath(new URL('../web/', import.meta.url));
const TYPES = {
    '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript',
    '.wasm': 'application/wasm', '.json': 'application/json', '.css': 'text/css',
    '.gif': 'image/gif', '.png': 'image/png', '.svg': 'image/svg+xml', '.ico': 'image/x-icon',
};

// Shared by local development, browser tests, and browser benchmarks.
export async function serve({ root = SITE_ROOT, port = 0, isolated = true, failThreaded = false } = {}) {
    root = await realpath(root);
    const server = createServer(async (request, response) => {
        if (isolated) {
            response.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
            response.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
            response.setHeader('Document-Isolation-Policy', 'isolate-and-require-corp');
        }
        response.setHeader('Access-Control-Allow-Origin', '*');
        try {
            if (!['GET', 'HEAD'].includes(request.method)) {
                response.writeHead(405).end();
                return;
            }
            const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
            if (failThreaded && pathname.includes('/pkg-threaded/')) {
                response.writeHead(503).end('Test: parallel package unavailable');
                return;
            }
            let path = resolve(root, `.${pathname}`);
            if (path !== root && !path.startsWith(root + sep)) {
                response.writeHead(403).end();
                return;
            }
            if ((await stat(path)).isDirectory()) path = resolve(path, 'index.html');
            path = await realpath(path);
            if (!path.startsWith(root + sep)) {
                response.writeHead(403).end();
                return;
            }
            const body = await readFile(path);
            response.writeHead(200, {
                'Content-Type': TYPES[extname(path)] ?? 'application/octet-stream',
                'Content-Length': body.length,
            });
            response.end(request.method === 'HEAD' ? undefined : body);
        } catch (error) {
            const status = error instanceof URIError ? 400
                : ['ENOENT', 'ENOTDIR'].includes(error.code) ? 404 : 500;
            response.writeHead(status).end();
        }
    });
    await new Promise((accept, reject) => {
        server.once('error', reject);
        server.listen(port, '127.0.0.1', accept);
    });
    return {
        url: `http://127.0.0.1:${server.address().port}`,
        close: () => new Promise((accept, reject) => {
            server.close(error => error ? reject(error) : accept());
            server.closeAllConnections();
        }),
    };
}

if (import.meta.main) {
    const { values } = parseArgs({ options: {
        port: { type: 'string', default: '8000' }, help: { type: 'boolean', short: 'h' },
    } });
    if (values.help) {
        console.log('Usage: npm run serve -- [--port 8000]');
    } else {
        const port = Number(values.port);
        if (!Number.isInteger(port) || port < 0 || port > 65535) throw new Error('Invalid port');
        const server = await serve({ port });
        console.log(`Open ${server.url}/`);
        for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => server.close());
    }
}
