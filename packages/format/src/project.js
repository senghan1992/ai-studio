import { promises as fs } from 'node:fs';
import path from 'node:path';

import { newProjectId, slugify, pad } from './ids.js';
import { readSlide, writeSlide, makeSlide, normalizeSlide } from './deck.js';
import { readSection, writeSection, makeSection, normalizeSection } from './doc.js';
import { readSheet, writeSheet, makeSheet, normalizeSheet } from './grid.js';
import { buildDigest } from './digest.js';

export const PROJECT_TYPES = ['deck', 'doc', 'grid'];

export const TYPE_INFO = {
  deck: { ext: '.aideck', dir: 'slides', key: 'slides', label: '프레젠테이션', format: 'ai-studio/deck' },
  doc: { ext: '.aidoc', dir: 'content', key: 'sections', label: '문서', format: 'ai-studio/doc' },
  grid: { ext: '.aigrid', dir: 'sheets', key: 'sheets', label: '스프레드시트', format: 'ai-studio/grid' },
};

export const DEFAULT_THEME = { name: 'aurora', accent: '#4f46e5', font: 'Inter' };

const EXT_TO_TYPE = Object.fromEntries(Object.entries(TYPE_INFO).map(([t, i]) => [i.ext, t]));

export function typeFromPath(dir) {
  return EXT_TO_TYPE[path.extname(dir)] ?? null;
}

/* --------------------------------------------------------------- utilities */

async function readJson(file, fallback = null) {
  try {
    return JSON.parse(await fs.readFile(file, 'utf8'));
  } catch {
    return fallback;
  }
}

async function readText(file, fallback = '') {
  try {
    return await fs.readFile(file, 'utf8');
  } catch {
    return fallback;
  }
}

async function writeJson(file, data) {
  await fs.mkdir(path.dirname(file), { recursive: true });
  await fs.writeFile(file, `${JSON.stringify(data, null, 2)}\n`, 'utf8');
}

async function writeText(file, text) {
  await fs.mkdir(path.dirname(file), { recursive: true });
  await fs.writeFile(file, text, 'utf8');
}

async function exists(p) {
  try {
    await fs.access(p);
    return true;
  } catch {
    return false;
  }
}

/** Reject paths that would escape the workspace root. */
export function resolveInside(root, target) {
  const abs = path.resolve(root, target);
  const rel = path.relative(path.resolve(root), abs);
  if (rel.startsWith('..') || path.isAbsolute(rel)) {
    throw new Error(`경로가 작업 폴더를 벗어납니다: ${target}`);
  }
  return abs;
}

/* ------------------------------------------------------------------ create */

export async function createProject(root, { type, title, sample = true } = {}) {
  if (!PROJECT_TYPES.includes(type)) throw new Error(`알 수 없는 문서 종류: ${type}`);
  const info = TYPE_INFO[type];
  const name = (title ?? '').trim() || `제목 없는 ${info.label}`;

  const dir = await uniqueDir(root, `${slugify(name, 'untitled')}${info.ext}`);
  await fs.mkdir(path.join(dir, 'assets'), { recursive: true });

  const now = new Date().toISOString();
  const project = {
    type,
    dir,
    manifest: {
      format: info.format,
      formatVersion: 1,
      id: newProjectId(),
      title: name,
      created: now,
      modified: now,
      theme: { ...DEFAULT_THEME },
    },
  };

  if (type === 'deck') {
    project.slides = sample
      ? [makeSlide('title', { title: name }), makeSlide('title-content', { title: '개요', index: 2 })]
      : [makeSlide('title', { title: name })];
  } else if (type === 'doc') {
    project.sections = [makeSection({ name: name })];
  } else {
    project.sheets = [makeSheet({ name: '시트1', withSample: sample })];
  }

  await saveProject(project);
  return loadProject(dir);
}

async function uniqueDir(root, base) {
  const target = path.join(root, base);
  if (!(await exists(target))) return target;
  const ext = path.extname(base);
  const stem = base.slice(0, -ext.length);
  for (let i = 2; i < 1000; i++) {
    const candidate = path.join(root, `${stem}-${i}${ext}`);
    if (!(await exists(candidate))) return candidate;
  }
  throw new Error('사용 가능한 폴더 이름을 찾지 못했습니다');
}

/* -------------------------------------------------------------------- load */

export async function loadProject(dir) {
  const type = typeFromPath(dir);
  if (!type) throw new Error(`AI Studio 프로젝트가 아닙니다: ${path.basename(dir)}`);
  const info = TYPE_INFO[type];

  const manifest = (await readJson(path.join(dir, 'manifest.json'))) ?? {
    format: info.format,
    formatVersion: 1,
    id: newProjectId(),
    title: path.basename(dir, info.ext),
    created: new Date().toISOString(),
    modified: new Date().toISOString(),
    theme: { ...DEFAULT_THEME },
  };

  const entries = await resolveEntries(dir, type, manifest);
  const project = { type, dir, manifest };

  if (type === 'deck') {
    project.slides = await Promise.all(
      entries.map(async (e) => {
        const md = await readText(path.join(dir, e.md));
        const layout = await readJson(path.join(dir, e.json));
        return { ...readSlide({ md, layout }), file: e.md };
      })
    );
    if (!project.slides.length) project.slides = [makeSlide('title', { title: manifest.title })];
  } else if (type === 'doc') {
    project.sections = await Promise.all(
      entries.map(async (e) => {
        const md = await readText(path.join(dir, e.md));
        const meta = await readJson(path.join(dir, e.json));
        return { ...readSection({ md, meta }), file: e.md };
      })
    );
    if (!project.sections.length) project.sections = [makeSection({ name: manifest.title })];
  } else {
    project.sheets = await Promise.all(
      entries.map(async (e) => {
        const md = await readText(path.join(dir, e.md));
        const cells = await readJson(path.join(dir, e.json));
        return { ...readSheet({ md, cells }), file: e.md };
      })
    );
    if (!project.sheets.length) project.sheets = [makeSheet({ name: '시트1' })];
  }

  return project;
}

/**
 * Work out which md/json pairs belong to the project.
 *
 * The manifest is the ordering authority, but a directory scan fills in files
 * added by hand (or by an AI agent writing markdown directly) so they are not
 * silently ignored.
 */
async function resolveEntries(dir, type, manifest) {
  const info = TYPE_INFO[type];
  const listed = Array.isArray(manifest[info.key]) ? manifest[info.key] : [];
  const entries = [];
  const seen = new Set();

  for (const item of listed) {
    if (!item?.md) continue;
    if (!(await exists(path.join(dir, item.md)))) continue;
    entries.push({ md: item.md, json: item.json ?? jsonPathFor(type, item.md) });
    seen.add(path.normalize(item.md));
  }

  let names = [];
  try {
    names = await fs.readdir(path.join(dir, info.dir));
  } catch {
    names = [];
  }
  const extras = names
    .filter((n) => n.endsWith('.md'))
    .map((n) => path.posix.join(info.dir, n))
    .filter((rel) => !seen.has(path.normalize(rel)))
    .sort();

  for (const rel of extras) entries.push({ md: rel, json: jsonPathFor(type, rel) });
  return entries;
}

function jsonPathFor(type, mdPath) {
  const suffix = type === 'deck' ? '.layout.json' : type === 'doc' ? '.meta.json' : '.cells.json';
  return mdPath.replace(/\.md$/, suffix);
}

/* -------------------------------------------------------------------- save */

/**
 * Write a project to disk as md/json pairs, then regenerate AI.md.
 *
 * Files are renumbered to match the current order so `ls` reflects the deck order,
 * and stale files from removed or renamed items are pruned.
 */
export async function saveProject(project) {
  const { type, dir } = project;
  const info = TYPE_INFO[type];
  if (!info) throw new Error(`알 수 없는 문서 종류: ${type}`);

  const items = normalizeItems(project);
  const written = [];
  const keep = new Set();

  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    const stem = `${pad(i + 1)}-${slugify(itemName(type, item), `item-${i + 1}`)}`;
    const mdRel = path.posix.join(info.dir, `${stem}.md`);
    const jsonRel = jsonPathFor(type, mdRel);

    const pair = serializeItem(type, item);
    await writeText(path.join(dir, mdRel), pair.md);
    await writeJson(path.join(dir, jsonRel), pair.json);

    keep.add(path.basename(mdRel));
    keep.add(path.basename(jsonRel));
    written.push({ id: item.id, name: itemName(type, item), md: mdRel, json: jsonRel });
  }

  await pruneDir(path.join(dir, info.dir), keep);

  const manifest = {
    ...project.manifest,
    format: info.format,
    formatVersion: 1,
    id: project.manifest?.id ?? newProjectId(),
    title: project.manifest?.title ?? path.basename(dir, info.ext),
    created: project.manifest?.created ?? new Date().toISOString(),
    modified: new Date().toISOString(),
    theme: { ...DEFAULT_THEME, ...(project.manifest?.theme ?? {}) },
    [info.key]: written,
  };
  await writeJson(path.join(dir, 'manifest.json'), manifest);

  const saved = { ...project, manifest, [info.key]: items };
  await writeText(path.join(dir, 'AI.md'), buildDigest(saved));

  return saved;
}

function normalizeItems(project) {
  switch (project.type) {
    case 'deck':
      return (project.slides ?? []).map(normalizeSlide);
    case 'doc':
      return (project.sections ?? []).map(normalizeSection);
    default:
      return (project.sheets ?? []).map(normalizeSheet);
  }
}

function serializeItem(type, item) {
  if (type === 'deck') {
    const { md, layout } = writeSlide(item);
    return { md, json: layout };
  }
  if (type === 'doc') {
    const { md, meta } = writeSection(item);
    return { md, json: meta };
  }
  const { md, cells } = writeSheet(item);
  return { md, json: cells };
}

function itemName(type, item) {
  if (type === 'deck') return item.title || '슬라이드';
  if (type === 'doc') return item.name || '섹션';
  return item.name || '시트';
}

async function pruneDir(dir, keep) {
  let names = [];
  try {
    names = await fs.readdir(dir);
  } catch {
    return;
  }
  await Promise.all(
    names
      .filter((n) => (n.endsWith('.md') || n.endsWith('.json')) && !keep.has(n))
      .map((n) => fs.rm(path.join(dir, n), { force: true }))
  );
}

/* -------------------------------------------------------------------- list */

export async function listProjects(root) {
  await fs.mkdir(root, { recursive: true });
  const names = await fs.readdir(root, { withFileTypes: true });
  const out = [];

  for (const entry of names) {
    if (!entry.isDirectory()) continue;
    const type = typeFromPath(entry.name);
    if (!type) continue;
    const dir = path.join(root, entry.name);
    const manifest = await readJson(path.join(dir, 'manifest.json'));
    const info = TYPE_INFO[type];
    const stat = await fs.stat(dir).catch(() => null);
    out.push({
      type,
      folder: entry.name,
      title: manifest?.title ?? path.basename(entry.name, info.ext),
      id: manifest?.id ?? null,
      modified: manifest?.modified ?? stat?.mtime?.toISOString() ?? null,
      count: Array.isArray(manifest?.[info.key]) ? manifest[info.key].length : 0,
      label: info.label,
    });
  }

  out.sort((a, b) => String(b.modified ?? '').localeCompare(String(a.modified ?? '')));
  return out;
}

export async function renameProject(root, folder, title) {
  const dir = resolveInside(root, folder);
  const type = typeFromPath(dir);
  if (!type) throw new Error('AI Studio 프로젝트가 아닙니다');
  const info = TYPE_INFO[type];

  const manifest = (await readJson(path.join(dir, 'manifest.json'))) ?? {};
  manifest.title = title;
  manifest.modified = new Date().toISOString();
  await writeJson(path.join(dir, 'manifest.json'), manifest);

  const nextDir = await uniqueDir(root, `${slugify(title, 'untitled')}${info.ext}`);
  if (path.resolve(nextDir) !== path.resolve(dir)) {
    await fs.rename(dir, nextDir);
    const project = await loadProject(nextDir);
    await saveProject(project);
    return path.basename(nextDir);
  }
  const project = await loadProject(dir);
  await saveProject(project);
  return folder;
}

export async function deleteProject(root, folder) {
  const dir = resolveInside(root, folder);
  if (!typeFromPath(dir)) throw new Error('AI Studio 프로젝트가 아닙니다');
  await fs.rm(dir, { recursive: true, force: true });
}

/** Every file in a project, for the "파일 보기" inspector in the UI. */
export async function projectFiles(dir) {
  const out = [];
  const walk = async (current, rel = '') => {
    const entries = await fs.readdir(current, { withFileTypes: true });
    for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
      const abs = path.join(current, entry.name);
      const relPath = rel ? path.posix.join(rel, entry.name) : entry.name;
      if (entry.isDirectory()) {
        await walk(abs, relPath);
      } else {
        const stat = await fs.stat(abs).catch(() => null);
        out.push({ path: relPath, size: stat?.size ?? 0 });
      }
    }
  };
  await walk(dir);
  return out;
}

export async function readProjectFile(dir, relPath) {
  const abs = resolveInside(dir, relPath);
  return readText(abs);
}
