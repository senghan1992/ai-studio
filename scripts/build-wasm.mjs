#!/usr/bin/env node
/**
 * Compile the Rust core to WebAssembly for the editor.
 *
 * `cargo build` produces the module; `wasm-bindgen` writes the JS shim that
 * marshals values across the boundary. Both must be present — this script says
 * so plainly rather than letting Vite fail with a missing import later.
 */
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const outDir = path.join(root, 'apps/web/src/core/pkg');
const wasmPath = path.join(
  root,
  'target/wasm32-unknown-unknown/wasm-release/ai_studio_wasm.wasm'
);

function run(command, args, label) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit' });
  if (result.error?.code === 'ENOENT') {
    console.error(`\n${command} 을 찾을 수 없습니다. ${label}`);
    process.exit(1);
  }
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run(
  'cargo',
  [
    'build',
    '--profile', 'wasm-release',
    '-p', 'ai-studio-wasm',
    '--target', 'wasm32-unknown-unknown',
  ],
  'Rust 툴체인이 필요합니다: https://rustup.rs  (그 다음 rustup target add wasm32-unknown-unknown)'
);

mkdirSync(outDir, { recursive: true });
run(
  'wasm-bindgen',
  ['--target', 'web', '--out-dir', outDir, '--no-typescript', wasmPath],
  'cargo install wasm-bindgen-cli 로 설치하세요.'
);

// `wasm-opt` is optional; it typically takes another 20% off the module.
const optimised = spawnSync('wasm-opt', ['--version'], { stdio: 'ignore' });
const bundle = path.join(outDir, 'ai_studio_wasm_bg.wasm');
if (optimised.status === 0) {
  run('wasm-opt', ['-Oz', '-o', bundle, bundle], '');
}

if (!existsSync(bundle)) {
  console.error('wasm 번들이 생성되지 않았습니다.');
  process.exit(1);
}
const kb = (statSync(bundle).size / 1024).toFixed(0);
console.log(`코어 준비 완료: ${path.relative(root, bundle)} (${kb} KB)`);
