// The coordinator also runs preparation, keeping all Rust work off the UI thread.
let api;

self.onmessage = async ({ data }) => {
    try {
        if (data.type === 'init') {
            api = await import(data.threaded
                ? './pkg-threaded/shapeshifter.js' : './pkg/shapeshifter.js');
            const wasm = await api.default();
            if (data.threaded) await api.initThreadPool(data.threads);
            self.postMessage({
                type: 'ready', workers: api.worker_count(),
                memory: data.threaded ? wasm.memory.buffer : null,
                cancellationPtr: data.threaded ? api.cancellation_ptr() : null,
            });
        } else if (data.type === 'solve') {
            let search;
            try {
                self.postMessage({ type: 'preparing', id: data.id });
                const start = performance.now();
                search = new api.BrowserSearch(data.json);
                const preparation_ms = performance.now() - start;
                self.postMessage({ type: 'searching', id: data.id, preparation_ms });
                const result = JSON.parse(search.solve(data.budgetMs));
                self.postMessage({ type: 'result', id: data.id, result: { ...result, preparation_ms } });
            } finally {
                search?.free();
            }
        }
    } catch (error) {
        self.postMessage({ type: 'error', id: data.id,
            message: String(error?.message ?? error), fatal: error instanceof WebAssembly.RuntimeError });
    }
};
