import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const API_TARGET = process.env.AI_STUDIO_API ?? 'http://localhost:5177';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5178,
    strictPort: true,
    // In browser mode the UI talks to `ai-studio-serve`. Inside the desktop app
    // there is no server at all — `api.js` routes through Tauri commands.
    proxy: { '/api': { target: API_TARGET, changeOrigin: true } },
  },
  // The wasm core is imported with `?url` and instantiated at runtime, so it
  // must stay a real asset rather than being inlined as base64.
  assetsInclude: ['**/*.wasm'],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    assetsInlineLimit: 0,
    target: 'es2022',
  },
});
