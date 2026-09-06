// A single reusable pool owns one puzzle at a time. No puzzle data leaves the browser.
export class SolverClient {
    constructor({ onStatus = () => {}, threads = navigator.hardwareConcurrency || 1 } = {}) {
        this.onStatus = onStatus;
        this.threads = Math.max(1, Math.floor(threads));
        this.nextId = 0;
        this.worker = null;
        this.ready = null;
        this.pending = null;
    }

    init() {
        if (!this.ready) {
            const threaded = self.crossOriginIsolated && typeof SharedArrayBuffer !== 'undefined';
            this.ready = this.startWorker(threaded).catch(async error => {
                if (error.name === 'AbortError') throw error;
                this.stopWorker(false);
                if (!threaded) throw error;
                this.onStatus({ type: 'fallback', message: error.message });
                return this.startWorker(false);
            }).catch(error => {
                if (error.name !== 'AbortError') this.stopWorker();
                throw error;
            });
        }
        return this.ready;
    }

    startWorker(threaded) {
        this.onStatus({ type: 'loading' });
        return new Promise((resolve, reject) => {
            const worker = new Worker(new URL('./search-worker.js', import.meta.url), { type: 'module' });
            this.worker = worker;
            this.startupReject = reject;
            let initialized = false;
            const timer = setTimeout(() => reject(new Error('Solver startup timed out')), 30000);
            this.startupTimer = timer;
            worker.onmessage = ({ data }) => {
                if (worker !== this.worker) return;
                if (data.type === 'ready') {
                    clearTimeout(timer);
                    initialized = true;
                    this.startupReject = null;
                    this.cancellation = data.memory
                        ? new Int32Array(data.memory, data.cancellationPtr, 1) : null;
                    this.info = { workers: data.workers, threaded };
                    this.onStatus({ type: 'ready', ...this.info });
                    resolve(this.info);
                } else if (data.type === 'error' && !initialized) {
                    clearTimeout(timer);
                    reject(new Error(data.message));
                } else {
                    this.handleMessage(data);
                }
            };
            worker.onerror = event => {
                clearTimeout(timer);
                const error = new Error(event.message || 'Solver worker failed');
                if (!initialized) reject(error);
                else {
                    this.finish(null, error);
                    this.stopWorker();
                }
            };
            worker.postMessage({ type: 'init', threaded, threads: this.threads });
        });
    }

    solve(puzzle, budgetMs = 120000) {
        if (this.pending) return Promise.reject(new Error('A puzzle is already running'));
        // Reserve the slot before startup, so concurrent callers cannot overlap.
        return new Promise((resolve, reject) => {
            const job = { id: ++this.nextId, resolve, reject,
                budgetMs: Math.max(0, Math.min(120000, Math.floor(budgetMs))) };
            this.pending = job;
            this.init().then(() => {
                if (this.pending !== job) return;
                if (this.cancellation) Atomics.store(this.cancellation, 0, 0);
                this.worker.postMessage({ type: 'solve', id: job.id, json: JSON.stringify(puzzle), budgetMs: job.budgetMs });
            }).catch(error => {
                if (this.pending === job) this.finish(null, error);
            });
        });
    }

    handleMessage(data) {
        const job = this.pending;
        if (!job || data.id !== job.id) return;
        if (data.type === 'result') {
            this.finish(data.result);
        } else if (data.type === 'error') {
            this.finish(null, new Error(data.message));
            if (data.fatal) this.stopWorker();
        } else {
            if (data.type === 'searching') {
                // Rust enforces the search cap. This also recovers a trapped/unresponsive worker.
                job.watchdog = setTimeout(() => {
                    this.finish(null, new Error('Solver stopped responding at the search deadline'));
                    this.stopWorker();
                }, job.budgetMs + 5000);
            }
            this.onStatus({ ...data, ...this.info });
        }
    }

    cancel() {
        const job = this.pending;
        if (!job || job.cancelled) return;
        job.cancelled = true;
        if (this.cancellation) {
            Atomics.store(this.cancellation, 0, 1);
            // Preparation can be inside a long table build. Termination remains available.
            job.cancelTimer = setTimeout(() => {
                this.finish({ solved: false, cancelled: true });
                this.stopWorker();
            }, 3000);
        } else {
            this.finish({ solved: false, cancelled: true });
            this.stopWorker();
        }
    }

    finish(result, error) {
        const job = this.pending;
        if (!job) return;
        clearTimeout(job.watchdog);
        clearTimeout(job.cancelTimer);
        this.pending = null;
        if (job.cancelled) job.resolve?.({ ...result, solved: false, cancelled: true });
        else if (error) job.reject?.(error);
        else job.resolve?.(result);
    }

    stopWorker(resetReady = true) {
        clearTimeout(this.startupTimer);
        this.startupReject?.(new DOMException('Solver startup cancelled', 'AbortError'));
        this.startupReject = null;
        this.worker?.terminate();
        this.worker = null;
        if (resetReady) this.ready = null;
        this.cancellation = null;
    }

    dispose() {
        this.finish({ solved: false, cancelled: true });
        this.stopWorker();
    }
}
