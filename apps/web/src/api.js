/**
 * The document API, over whichever transport this build is running on.
 *
 * Inside the desktop app there is no HTTP server: Tauri exposes the same
 * `ai-core` methods as commands, and `invoke` calls them directly over IPC. In a
 * browser the identical methods arrive over `/api` from `ai-studio-serve`. The
 * two shapes are the same because both front ends are thin wrappers over one
 * Rust implementation, so nothing above this file knows which is in use.
 */

/** True when running inside the Tauri shell rather than a plain browser. */
export const isDesktop = () =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

let invokeFn = null;

/** Load Tauri's `invoke` lazily, so a browser build never pulls it in. */
async function invoke(command, args) {
  if (!invokeFn) {
    const mod = await import('@tauri-apps/api/core');
    invokeFn = mod.invoke;
  }
  return invokeFn(command, args);
}

const BASE = '/api';
const enc = encodeURIComponent;

async function request(path, options = {}) {
  const res = await fetch(`${BASE}${path}`, {
    headers: options.body ? { 'Content-Type': 'application/json' } : undefined,
    ...options,
  });
  if (!res.ok) {
    let message = `요청 실패 (${res.status})`;
    try {
      const data = await res.json();
      if (data?.error) message = data.error;
    } catch {
      /* non-JSON error body */
    }
    throw new Error(message);
  }
  if (res.status === 204) return null;
  const type = res.headers.get('content-type') ?? '';
  return type.includes('application/json') ? res.json() : res.text();
}

/**
 * One call, either transport.
 *
 * `command` is the Tauri command name; `http` describes the same call as a
 * request. Keeping them side by side is what stops the two from drifting.
 */
function call(command, args, http) {
  return isDesktop() ? invoke(command, args) : request(http.path, http.options);
}

const json = (body) => ({ method: body.method ?? 'POST', body: JSON.stringify(body.data) });

export const api = {
  health: () => call('health', {}, { path: '/health' }),

  listProjects: () => call('list_projects', {}, { path: '/projects' }),

  createProject: (type, title, sample = true) =>
    call(
      'create_project',
      { request: { type, title, sample } },
      { path: '/projects', options: json({ data: { type, title, sample } }) }
    ),

  getProject: (folder) =>
    call('get_project', { folder }, { path: `/projects/${enc(folder)}` }),

  saveProject: (folder, project) =>
    call(
      'save_project',
      { folder, payload: project },
      { path: `/projects/${enc(folder)}`, options: json({ method: 'PUT', data: project }) }
    ),

  renameProject: (folder, title) =>
    call(
      'rename_project',
      { folder, title },
      { path: `/projects/${enc(folder)}`, options: json({ method: 'PATCH', data: { title } }) }
    ),

  deleteProject: (folder) =>
    call(
      'delete_project',
      { folder },
      { path: `/projects/${enc(folder)}`, options: { method: 'DELETE' } }
    ),

  listFiles: (folder) =>
    call('project_files', { folder }, { path: `/projects/${enc(folder)}/files` }),

  readFile: (folder, path) =>
    call(
      'read_file',
      { folder, path },
      { path: `/projects/${enc(folder)}/file?path=${enc(path)}` }
    ),

  digest: (folder) =>
    call('digest', { folder }, { path: `/projects/${enc(folder)}/digest` }),

  listAssets: (folder) =>
    call('list_assets', { folder }, { path: `/projects/${enc(folder)}/assets` }),

  uploadAsset: (folder, name, dataUrl) =>
    call(
      'upload_asset',
      { folder, request: { name, dataUrl } },
      {
        path: `/projects/${enc(folder)}/assets`,
        options: json({ data: { name, dataUrl } }),
      }
    ),

  /** Recalculate a sheet on the native side. The editors normally use the wasm
   *  core directly; this is here for parity and for very large sheets. */
  recalc: (cells, names) =>
    call(
      'recalc',
      { request: { cells, names } },
      { path: '/recalc', options: json({ data: { cells, names } }) }
    ),
};

/**
 * Save an export to a file the user chooses.
 *
 * In the desktop app the native side writes the bytes and returns the path it
 * used; in a browser the download comes from the server as a normal navigation.
 */
export async function exportProject(folder, ext) {
  if (isDesktop()) {
    return invoke('export_project', { folder, ext });
  }
  // A plain navigation lets the browser handle the Content-Disposition header.
  window.location.assign(`${BASE}/projects/${enc(folder)}/export/${enc(ext)}`);
  return null;
}

/**
 * URL that displays an image stored in a project.
 *
 * Markdown keeps a relative path (`../assets/chart.png`) so the folder stays
 * portable; the editor needs something an `<img>` can load. In the desktop app
 * that is a custom `aistudio://` scheme the Rust side answers with the same
 * containment check the HTTP route uses; in a browser it is that HTTP route.
 */
export function assetUrl(folder, relPath) {
  const clean = String(relPath ?? '').replace(/^\.\.\//, '').replace(/^\.\//, '');
  if (!isDesktop()) {
    return `${BASE}/projects/${enc(folder)}/asset?path=${enc(clean)}`;
  }
  const target = `${enc(folder)}/${clean.split('/').map(enc).join('/')}`;
  const internals = window.__TAURI_INTERNALS__;
  if (typeof internals?.convertFileSrc === 'function') {
    return internals.convertFileSrc(target, 'aistudio');
  }
  // Same construction Tauri uses, for a shell that predates convertFileSrc.
  return navigator.userAgent.includes('Windows')
    ? `http://aistudio.localhost/${target}`
    : `aistudio://localhost/${target}`;
}

/** True for a path that lives inside the project rather than on the web. */
export function isProjectAsset(src) {
  const value = String(src ?? '');
  if (/^(https?:|data:|\/\/)/i.test(value)) return false;
  return /(^|\/)assets\//.test(value);
}
