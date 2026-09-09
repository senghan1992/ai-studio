import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { register } from 'node:module';
import { transformSync } from 'esbuild';

/*
 * `node --test` load hooks: transpile .jsx on the fly so component tests can
 * import the editors directly. Registered from the root package.json:
 * `node --test --import ./apps/web/test/loader.mjs`. The imported module must
 * register its own hooks — --import alone does not activate them.
 */
if (!globalThis.__jsxLoaderRegistered) {
  globalThis.__jsxLoaderRegistered = true;
  register(new URL(import.meta.url), { parentURL: import.meta.url });
}

export async function load(url, context, nextLoad) {
  if (url.startsWith('file:') && fileURLToPath(url).endsWith('.jsx')) {
    const source = await readFile(fileURLToPath(url), 'utf8');
    const { code } = transformSync(source, {
      loader: 'jsx',
      jsx: 'automatic',
      format: 'esm',
      target: 'node20',
      sourcefile: url,
      sourcemap: 'inline',
    });
    return { format: 'module', source: code, shortCircuit: true };
  }
  return nextLoad(url, context);
}