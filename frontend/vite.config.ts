import { readFileSync } from 'node:fs';
import { defineConfig, type ProxyOptions } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { parse } from 'smol-toml';

// market_api accepts browser requests only from server.public_url of config.toml.
// The dev server has its own origin (127.0.0.1:5173), so requests it forwards carry
// public_url instead; it listens on 127.0.0.1 only.
function proxy() {
  const config = parse(readFileSync(process.env.AEZA_CONFIG ?? new URL('../config.toml', import.meta.url), 'utf8')) as { server: { public_url: string } };
  const origin = new URL(config.server.public_url).origin;
  const api = (target: string, ws = false): ProxyOptions => ({
    target,
    ws,
    configure: proxy => {
      proxy.on('proxyReq', request => request.setHeader('origin', origin));
      proxy.on('proxyReqWs', request => request.setHeader('origin', origin));
    },
  });
  // Market Simulator (market-simulator crate) serves /api/market/v1 and /api/market/v2;
  // everything else under /api is market_api, including the /api/client/ws WebSocket.
  // Specific prefixes must come first.
  const simulator = { target: 'http://127.0.0.1:8091', changeOrigin: true };
  return { '/api/market/v1': simulator, '/api/market/v2': simulator, '/api/client/ws': api('ws://127.0.0.1:3000', true), '/api': api('http://127.0.0.1:3000') };
}

// Only the dev and preview servers proxy; `vite build` (also inside the frontend
// image, which has no config.toml) does not read the configuration.
export default defineConfig(({ command }) => {
  const servers = command === 'serve' ? { server: { proxy: proxy() }, preview: { proxy: proxy() } } : {};
  return { plugins: [react(), tailwindcss()], ...servers };
});
