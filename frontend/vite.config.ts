import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
// Market Simulator (market-simulator crate) serves /api/market/v1 and /api/market/v2;
// everything else under /api is market_api, including the /api/client/ws WebSocket.
// Specific prefixes must come first.
const simulator = { target: 'http://127.0.0.1:8091', changeOrigin: true };
const proxy = { '/api/market/v1': simulator, '/api/market/v2': simulator, '/api/client/ws': { target: 'ws://127.0.0.1:3000', ws: true }, '/api': 'http://127.0.0.1:3000' };
export default defineConfig({ plugins: [react(), tailwindcss()], server: { proxy }, preview: { proxy } });
