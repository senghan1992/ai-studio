import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const API_TARGET = process.env.AI_STUDIO_API ?? 'http://localhost:5177';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5178,
    proxy: { '/api': { target: API_TARGET, changeOrigin: true } },
  },
  resolve: {
    alias: {
      // The workspace packages are plain ESM source; alias them so Vite bundles
      // the same parsers the server uses instead of a stale copy.
      '@ai-studio/format/browser': path.resolve(here, '../../packages/format/src/browser.js'),
      '@ai-studio/formula': path.resolve(here, '../../packages/formula/src/index.js'),
    },
  },
  build: { outDir: 'dist', emptyOutDir: true },
});
