const BASE = '/api';

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
  const type = res.headers.get('content-type') ?? '';
  return type.includes('application/json') ? res.json() : res.text();
}

const enc = encodeURIComponent;

export const api = {
  health: () => request('/health'),
  listProjects: () => request('/projects'),
  createProject: (type, title, sample = true) =>
    request('/projects', { method: 'POST', body: JSON.stringify({ type, title, sample }) }),
  getProject: (folder) => request(`/projects/${enc(folder)}`),
  saveProject: (folder, project) =>
    request(`/projects/${enc(folder)}`, { method: 'PUT', body: JSON.stringify(project) }),
  renameProject: (folder, title) =>
    request(`/projects/${enc(folder)}`, { method: 'PATCH', body: JSON.stringify({ title }) }),
  deleteProject: (folder) => request(`/projects/${enc(folder)}`, { method: 'DELETE' }),
  listFiles: (folder) => request(`/projects/${enc(folder)}/files`),
  readFile: (folder, path) => request(`/projects/${enc(folder)}/file?path=${enc(path)}`),
  digest: (folder) => request(`/projects/${enc(folder)}/digest`),

  listAssets: (folder) => request(`/projects/${enc(folder)}/assets`),
  uploadAsset: (folder, name, dataUrl) =>
    request(`/projects/${enc(folder)}/assets`, { method: 'POST', body: JSON.stringify({ name, dataUrl }) }),
};

/**
 * URL that serves an image stored in a project.
 *
 * Markdown keeps a relative path (`../assets/chart.png`) so the folder stays
 * portable; the editor needs an absolute URL to actually display it.
 */
export function assetUrl(folder, relPath) {
  const clean = String(relPath ?? '').replace(/^\.\.\//, '').replace(/^\.\//, '');
  return `${BASE}/projects/${enc(folder)}/asset?path=${enc(clean)}`;
}

/** True for a path that lives inside the project rather than on the web. */
export function isProjectAsset(src) {
  const value = String(src ?? '');
  if (/^(https?:|data:|\/\/)/i.test(value)) return false;
  return /(^|\/)assets\//.test(value);
}
