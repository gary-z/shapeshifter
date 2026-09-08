import { SolverClient } from './search-client.js';

let client;
window.addEventListener('message', async event => {
    if (event.source !== parent || !['https://www.neopets.com', 'https://neopets.com'].includes(event.origin)
        || event.data?.type !== 'shapeshifter:solve' || !event.ports[0] || client) return;
    const port = event.ports[0];
    client = new SolverClient({ onStatus: status => port.postMessage({ type: 'status', status }) });
    try {
        const result = await client.solve(event.data.puzzle, event.data.budgetMs);
        port.postMessage({ type: 'result', result });
    } catch (error) {
        port.postMessage({ type: 'error', message: error.message });
    } finally {
        client.dispose();
        port.close();
    }
});
window.addEventListener('pagehide', () => client?.dispose());
