import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

export function normalizePackages(packages) {
    for (const directory of packages) {
        for (const entry of readdirSync(directory, { withFileTypes: true })) {
            const path = join(directory, entry.name);
            if (entry.isDirectory()) normalizePackages([path]);
            else if (entry.isFile() && path.endsWith('.js')) {
                const text = readFileSync(path, 'utf8');
                if (text.includes('\r\n')) writeFileSync(path, text.replaceAll('\r\n', '\n'));
            }
        }
    }
}

if (import.meta.main) normalizePackages(['web/pkg', 'web/pkg-threaded']);
