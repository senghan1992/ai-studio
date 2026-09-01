/**
 * Load the Rust core for a Node test run.
 *
 * The browser gets the wasm binary over the network; here it comes off disk, so
 * the tests exercise the same module the editor does rather than a stand-in.
 */
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import { initCore } from '../src/core/index.js';

const wasmPath = fileURLToPath(
  new URL('../src/core/pkg/ai_studio_wasm_bg.wasm', import.meta.url)
);

let loaded = null;

export function loadCore() {
  if (!loaded) {
    loaded = readFile(wasmPath).then(
      (bytes) => initCore(bytes),
      () => {
        throw new Error(
          `wasm 코어가 없습니다: ${wasmPath}\nnpm run build:wasm 을 먼저 실행하세요.`
        );
      }
    );
  }
  return loaded;
}
