import { newBlockId } from './ids.js';

const BLOCK_MARKER = /^[ \t]*<!--[ \t]*block:[ \t]*([A-Za-z0-9_-]+)[ \t]*(?:\|([^>]*?))?-->[ \t]*$/;
const FENCE = /^[ \t]*(`{3,}|~{3,})/;

/**
 * Split a markdown body on `<!-- block:id -->` markers.
 *
 * Fenced code blocks are skipped so a marker inside a ``` fence stays content.
 * Text appearing before the first marker becomes an implicit block, which is what
 * makes hand-written markdown (no markers at all) openable.
 *
 * @returns {{id: string, md: string, implicit: boolean, hint: object}[]}
 */
export function splitBlocks(body) {
  const lines = String(body ?? '').split(/\r?\n/);
  const blocks = [];
  let current = null;
  let fence = null;
  const seen = new Set();

  const push = () => {
    if (!current) return;
    const md = current.lines.join('\n').replace(/^\s*\n+/, '').replace(/\s+$/, '');
    if (!current.implicit || md !== '') blocks.push({ ...current, md, lines: undefined });
    current = null;
  };

  for (const line of lines) {
    const f = line.match(FENCE);
    if (f) {
      if (!fence) fence = f[1][0].repeat(3);
      else if (line.trim().startsWith(fence)) fence = null;
    }

    const m = !fence && line.match(BLOCK_MARKER);
    if (m) {
      push();
      let id = m[1];
      // Duplicate ids in a hand-edited file would collapse two blocks into one.
      while (seen.has(id)) id = `${id}-${newBlockId().slice(2, 5)}`;
      seen.add(id);
      current = { id, lines: [], implicit: false, hint: parseHint(m[2]) };
      continue;
    }

    if (!current) {
      const id = newBlockId();
      seen.add(id);
      current = { id, lines: [], implicit: true, hint: {} };
    }
    current.lines.push(line);
  }
  push();

  return blocks;
}

/** `<!-- block:b1 | kind=image -->` -> `{ kind: 'image' }`. Optional sugar. */
function parseHint(raw) {
  const hint = {};
  if (!raw) return hint;
  for (const pair of raw.split(/[,;]/)) {
    const [k, v] = pair.split('=').map((s) => s?.trim());
    if (k && v) hint[k] = v;
  }
  return hint;
}

/** Rebuild a markdown body from blocks, emitting a marker for each. */
export function joinBlocks(blocks, { omitMarkerWhen } = {}) {
  const parts = [];
  for (const b of blocks) {
    const skipMarker = typeof omitMarkerWhen === 'function' && omitMarkerWhen(b);
    const md = String(b.md ?? '').replace(/\s+$/, '');
    // Marker and its content stay on adjacent lines so the file reads as prose.
    if (skipMarker) {
      if (md) parts.push(md);
    } else {
      parts.push(md ? `<!-- block:${b.id} -->\n${md}` : `<!-- block:${b.id} -->`);
    }
  }
  return parts.join('\n\n').replace(/\s+$/, '') + '\n';
}

/**
 * Guess a block's kind from its markdown, used when layout JSON has no entry
 * (a hand-added block) or when the user pastes content.
 */
export function inferKind(md) {
  const t = String(md ?? '').trim();
  if (!t) return 'text';
  if (/^```chart\b/m.test(t)) return 'chart';
  if (/^!\[[^\]]*\]\([^)]+\)\s*$/.test(t)) return 'image';
  if (/^\|.*\|\s*$/m.test(t) && /^\|[\s:|-]+\|\s*$/m.test(t)) return 'table';
  return 'text';
}

/** Extract the first heading or line of a block — used for outlines and digests. */
export function blockLabel(md, max = 80) {
  const t = String(md ?? '').trim();
  if (!t) return '';
  const heading = t.match(/^#{1,6}[ \t]+(.+)$/m);
  const raw = heading ? heading[1] : t.split('\n').find((l) => l.trim()) || '';
  const plain = raw
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/[*_`>#]/g, '')
    .replace(/^[-+*]\s+/, '')
    .trim();
  return plain.length > max ? `${plain.slice(0, max - 1)}…` : plain;
}
