import express from 'express';
import path from 'node:path';
import { promises as fs } from 'node:fs';
import { fileURLToPath } from 'node:url';

import {
  listProjects, createProject, loadProject, saveProject,
  renameProject, deleteProject, projectFiles, readProjectFile,
  resolveInside, typeFromPath, TYPE_INFO, PROJECT_TYPES,
} from '@ai-studio/format';
import { recalcSheet } from '@ai-studio/formula';
import { EXPORTERS } from './export.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '../../..');

const PORT = Number(process.env.PORT ?? 5177);
const WORKSPACE = path.resolve(process.env.AI_STUDIO_WORKSPACE ?? path.join(REPO_ROOT, 'workspace'));
const WEB_DIST = path.join(REPO_ROOT, 'apps/web/dist');

const app = express();
app.use(express.json({ limit: '32mb' }));

/** Wrap an async handler so a rejection becomes a 4xx/5xx instead of a hang. */
const route = (handler) => async (req, res) => {
  try {
    await handler(req, res);
  } catch (error) {
    const status = error.status ?? (/벗어납니다|아닙니다|알 수 없는/.test(error.message) ? 400 : 500);
    if (status >= 500) console.error(`[${req.method} ${req.path}]`, error);
    res.status(status).json({ error: error.message ?? '알 수 없는 오류' });
  }
};

/** Resolve a folder param to an absolute project dir, refusing traversal. */
function projectDir(folder) {
  const dir = resolveInside(WORKSPACE, folder);
  if (!typeFromPath(dir)) {
    const err = new Error('AI Studio 프로젝트가 아닙니다');
    err.status = 400;
    throw err;
  }
  return dir;
}

/**
 * Same, but also require the folder to exist.
 *
 * `loadProject` is deliberately forgiving — it fills in defaults so a
 * half-hand-written folder still opens — which means a typo'd name would
 * otherwise "succeed" and hand back an empty document.
 */
async function existingProjectDir(folder) {
  const dir = projectDir(folder);
  try {
    const stat = await fs.stat(dir);
    if (!stat.isDirectory()) throw new Error('not a directory');
  } catch {
    const err = new Error(`문서를 찾을 수 없습니다: ${folder}`);
    err.status = 404;
    throw err;
  }
  return dir;
}

/* ------------------------------------------------------------------ routes */

app.get('/api/health', (req, res) => {
  res.json({ ok: true, workspace: WORKSPACE, types: PROJECT_TYPES });
});

app.get('/api/projects', route(async (req, res) => {
  res.json({ workspace: WORKSPACE, projects: await listProjects(WORKSPACE) });
}));

app.post('/api/projects', route(async (req, res) => {
  const { type, title, sample = true } = req.body ?? {};
  if (!PROJECT_TYPES.includes(type)) {
    const err = new Error(`알 수 없는 문서 종류: ${type}`);
    err.status = 400;
    throw err;
  }
  await fs.mkdir(WORKSPACE, { recursive: true });
  const project = await createProject(WORKSPACE, { type, title, sample });
  res.status(201).json(toWire(project));
}));

app.get('/api/projects/:folder', route(async (req, res) => {
  const project = await loadProject(await existingProjectDir(req.params.folder));
  res.json(toWire(project));
}));

app.put('/api/projects/:folder', route(async (req, res) => {
  const dir = await existingProjectDir(req.params.folder);
  const incoming = req.body ?? {};
  const current = await loadProject(dir);
  const info = TYPE_INFO[current.type];

  // The client owns content; the server owns the directory and identity fields.
  const merged = {
    type: current.type,
    dir,
    manifest: {
      ...current.manifest,
      title: incoming.manifest?.title ?? current.manifest.title,
      theme: { ...current.manifest.theme, ...(incoming.manifest?.theme ?? {}) },
    },
    [info.key]: Array.isArray(incoming[info.key]) && incoming[info.key].length
      ? incoming[info.key]
      : current[info.key],
  };

  await saveProject(merged);
  const reloaded = await loadProject(dir);
  res.json(toWire(reloaded));
}));

app.patch('/api/projects/:folder', route(async (req, res) => {
  const title = String(req.body?.title ?? '').trim();
  if (!title) {
    const err = new Error('제목이 비어 있습니다');
    err.status = 400;
    throw err;
  }
  await existingProjectDir(req.params.folder);
  const folder = await renameProject(WORKSPACE, req.params.folder, title);
  const project = await loadProject(path.join(WORKSPACE, folder));
  res.json(toWire(project));
}));

app.delete('/api/projects/:folder', route(async (req, res) => {
  await existingProjectDir(req.params.folder);
  await deleteProject(WORKSPACE, req.params.folder);
  res.json({ ok: true });
}));

app.get('/api/projects/:folder/files', route(async (req, res) => {
  const dir = await existingProjectDir(req.params.folder);
  res.json({ files: await projectFiles(dir) });
}));

app.get('/api/projects/:folder/file', route(async (req, res) => {
  const dir = await existingProjectDir(req.params.folder);
  const rel = String(req.query.path ?? '');
  if (!rel) {
    const err = new Error('path 쿼리가 필요합니다');
    err.status = 400;
    throw err;
  }
  const content = await readProjectFile(dir, rel);
  res.json({ path: rel, content });
}));

app.get('/api/projects/:folder/digest', route(async (req, res) => {
  const dir = await existingProjectDir(req.params.folder);
  res.type('text/markdown').send(await readProjectFile(dir, 'AI.md'));
}));

/* --------------------------------------------------------------- assets */

const ASSET_TYPES = {
  'image/png': 'png',
  'image/jpeg': 'jpg',
  'image/gif': 'gif',
  'image/webp': 'webp',
  'image/svg+xml': 'svg',
};
const MAX_ASSET_BYTES = 12 * 1024 * 1024;

/**
 * Store an image inside the project's own `assets/` folder.
 *
 * Images live with the document rather than in a shared blob store, which is what
 * keeps a project a self-contained, copyable folder — and lets the markdown refer
 * to them with a plain relative path an AI agent can follow.
 */
app.post('/api/projects/:folder/assets', route(async (req, res) => {
  const dir = await existingProjectDir(req.params.folder);
  const { name = 'image', dataUrl = '' } = req.body ?? {};

  const match = String(dataUrl).match(/^data:([^;,]+);base64,(.+)$/s);
  if (!match) {
    const err = new Error('base64 data URL이 필요합니다');
    err.status = 400;
    throw err;
  }
  const ext = ASSET_TYPES[match[1].toLowerCase()];
  if (!ext) {
    const err = new Error(`지원하지 않는 이미지 형식입니다: ${match[1]}`);
    err.status = 400;
    throw err;
  }

  const buffer = Buffer.from(match[2], 'base64');
  if (!buffer.length) {
    const err = new Error('빈 파일입니다');
    err.status = 400;
    throw err;
  }
  if (buffer.length > MAX_ASSET_BYTES) {
    const err = new Error(`이미지가 너무 큽니다 (최대 ${Math.round(MAX_ASSET_BYTES / 1024 / 1024)}MB)`);
    err.status = 413;
    throw err;
  }

  const stem = path
    .basename(String(name), path.extname(String(name)))
    .replace(/[^\p{L}\p{N}_-]+/gu, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 48) || 'image';

  await fs.mkdir(path.join(dir, 'assets'), { recursive: true });
  let file = `${stem}.${ext}`;
  for (let i = 2; i < 500; i++) {
    try {
      await fs.access(path.join(dir, 'assets', file));
      file = `${stem}-${i}.${ext}`;
    } catch {
      break;
    }
  }

  await fs.writeFile(path.join(dir, 'assets', file), buffer);
  res.status(201).json({ path: `assets/${file}`, bytes: buffer.length });
}));

/** Serve an image from a project's assets folder. */
app.get('/api/projects/:folder/asset', route(async (req, res) => {
  const dir = await existingProjectDir(req.params.folder);
  const rel = String(req.query.path ?? '');
  // Only the assets folder is readable this way, and never through a traversal.
  const normalized = rel.replace(/^\.\.\//, '').replace(/^\.\//, '');
  if (!normalized.startsWith('assets/')) {
    const err = new Error('assets/ 안의 파일만 제공됩니다');
    err.status = 400;
    throw err;
  }
  const abs = resolveInside(dir, normalized);
  try {
    const stat = await fs.stat(abs);
    if (!stat.isFile()) throw new Error('not a file');
  } catch {
    const err = new Error('이미지를 찾을 수 없습니다');
    err.status = 404;
    throw err;
  }
  const ext = path.extname(abs).toLowerCase().slice(1);
  const mime = Object.entries(ASSET_TYPES).find(([, e]) => e === (ext === 'jpeg' ? 'jpg' : ext))?.[0];
  res.setHeader('Content-Type', mime ?? 'application/octet-stream');
  res.setHeader('Cache-Control', 'no-cache');
  res.sendFile(abs);
}));

app.get('/api/projects/:folder/assets', route(async (req, res) => {
  const dir = await existingProjectDir(req.params.folder);
  let names = [];
  try {
    names = await fs.readdir(path.join(dir, 'assets'));
  } catch {
    names = [];
  }
  const files = [];
  for (const name of names.sort()) {
    const stat = await fs.stat(path.join(dir, 'assets', name)).catch(() => null);
    if (stat?.isFile()) files.push({ path: `assets/${name}`, bytes: stat.size });
  }
  res.json({ assets: files });
}));

/**
 * Convert a project to an Office file.
 *
 * The conversion runs against the project as stored on disk, so what you export
 * is exactly what was saved — no separate in-memory representation to drift.
 */
app.get('/api/projects/:folder/export/:ext', route(async (req, res) => {
  const ext = String(req.params.ext).toLowerCase();
  const exporter = EXPORTERS[ext];
  if (!exporter) {
    const err = new Error(`지원하지 않는 형식입니다: ${ext}`);
    err.status = 400;
    throw err;
  }

  const dir = await existingProjectDir(req.params.folder);
  const project = await loadProject(dir);
  if (project.type !== exporter.type) {
    const err = new Error(`${TYPE_INFO[project.type].label}은(는) .${ext}로 내보낼 수 없습니다`);
    err.status = 400;
    throw err;
  }

  const buffer = await exporter.run(project);
  const filename = `${path.basename(dir, TYPE_INFO[project.type].ext)}.${ext}`;
  res.setHeader('Content-Type', exporter.mime);
  res.setHeader(
    'Content-Disposition',
    `attachment; filename="export.${ext}"; filename*=UTF-8''${encodeURIComponent(filename)}`
  );
  res.setHeader('Content-Length', String(buffer.length));
  res.end(buffer);
}));

/**
 * Recalculate a sheet server-side.
 * The browser runs the same engine for instant feedback; this exists so an API
 * client (or an AI agent) can get authoritative values without a browser.
 */
app.post('/api/recalc', route(async (req, res) => {
  const { cells = {}, names = {} } = req.body ?? {};
  res.json(recalcSheet({ cells, names }));
}));

/* ------------------------------------------------------------ static web */

app.use(express.static(WEB_DIST));
app.get(/^\/(?!api\/).*/, (req, res) => {
  res.sendFile(path.join(WEB_DIST, 'index.html'), (err) => {
    if (err) {
      res
        .status(200)
        .type('html')
        .send('<h1>AI Studio</h1><p>웹 앱이 아직 빌드되지 않았습니다. <code>npm run dev</code> 로 개발 서버를 실행하거나 <code>npm run build</code> 로 빌드하세요.</p>');
    }
  });
});

/** Strip absolute paths from the wire format; the client works with folders. */
function toWire(project) {
  const info = TYPE_INFO[project.type];
  return {
    type: project.type,
    folder: path.basename(project.dir),
    manifest: project.manifest,
    [info.key]: project[info.key],
  };
}

await fs.mkdir(WORKSPACE, { recursive: true });
app.listen(PORT, () => {
  console.log(`AI Studio 서버 실행 중 → http://localhost:${PORT}`);
  console.log(`작업 폴더: ${WORKSPACE}`);
});
