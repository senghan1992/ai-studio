import YAML from 'yaml';

const FM_RE = /^---\r?\n([\s\S]*?)\r?\n---[ \t]*\r?\n?/;
const BOM = '﻿';

/**
 * Split a markdown file into YAML frontmatter and body.
 * Missing or malformed frontmatter yields `{}` rather than throwing — hand-edited
 * files should still open.
 */
export function parseFrontmatter(text) {
  let src = String(text ?? '');
  if (src.startsWith(BOM)) src = src.slice(1);
  const m = src.match(FM_RE);
  if (!m) return { meta: {}, body: src.replace(/^\s*\n/, '') };
  let meta = {};
  try {
    meta = YAML.parse(m[1]) ?? {};
  } catch {
    meta = {};
  }
  if (typeof meta !== 'object' || Array.isArray(meta)) meta = {};
  return { meta, body: src.slice(m[0].length) };
}

/** Re-attach frontmatter. Empty meta emits no fence. */
export function serializeFrontmatter(meta, body) {
  const clean = {};
  for (const [k, v] of Object.entries(meta ?? {})) {
    if (v === undefined || v === null || v === '') continue;
    clean[k] = v;
  }
  const text = String(body ?? '').replace(/\s+$/, '');
  if (Object.keys(clean).length === 0) return text ? `${text}\n` : '';
  const yaml = YAML.stringify(clean, { lineWidth: 0 }).replace(/\n$/, '');
  return `---\n${yaml}\n---\n\n${text}\n`;
}
