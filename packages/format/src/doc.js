import { parseFrontmatter, serializeFrontmatter } from './frontmatter.js';
import { splitBlocks, joinBlocks } from './blocks.js';
import { splitMarkdownBlocks, headingLevel, plainText, countWords } from './mdblocks.js';
import { newSectionId, newBlockId } from './ids.js';

export const PAGE_SIZES = {
  A4: { w: 794, h: 1123 },
  Letter: { w: 816, h: 1056 },
  A5: { w: 559, h: 794 },
};

export const DEFAULT_PAGE = {
  size: 'A4',
  margin: { top: 72, right: 72, bottom: 72, left: 72 },
  columns: 1,
};

/** An override that equals the default carries no information, so we drop it. */
const DEFAULT_OVERRIDE = { align: 'left', indent: 0 };

/**
 * Merge a Doc section's markdown with its meta JSON.
 *
 * Doc markdown stays marker-free wherever formatting is default; only blocks with
 * an override carry a `<!-- block:id -->` anchor. Marker-free regions are
 * re-segmented into paragraph blocks so the editor has something to address.
 */
export function readSection({ md = '', meta: metaJson = null } = {}) {
  const { meta, body } = parseFrontmatter(md);
  const regions = splitBlocks(body);
  const overrides = metaJson?.blocks ?? {};

  const blocks = [];
  for (const region of regions) {
    if (region.implicit) {
      for (const part of splitMarkdownBlocks(region.md)) {
        blocks.push({ id: newBlockId(), md: part.md, type: part.type, override: null });
      }
    } else {
      const o = overrides[region.id] ?? null;
      const parts = splitMarkdownBlocks(region.md);
      // An anchored region should stay one block; if markdown made it several,
      // the anchor applies to the first and the rest become plain blocks.
      parts.forEach((part, i) => {
        blocks.push({
          id: i === 0 ? region.id : newBlockId(),
          md: part.md,
          type: part.type,
          override: i === 0 ? normalizeOverride(o) : null,
        });
      });
      if (parts.length === 0) blocks.push({ id: region.id, md: '', type: 'paragraph', override: normalizeOverride(o) });
    }
  }

  return {
    id: meta.id || metaJson?.id || newSectionId(),
    name: meta.name ?? meta.title ?? '섹션',
    page: { ...DEFAULT_PAGE, ...(metaJson?.page ?? {}) },
    blocks,
  };
}

/** Split a section back into the markdown + meta JSON pair. */
export function writeSection(section) {
  const s = normalizeSection(section);
  const body = joinBlocks(s.blocks, { omitMarkerWhen: (b) => !b.override });
  const md = serializeFrontmatter({ id: s.id, name: s.name }, body);

  const blocks = {};
  for (const b of s.blocks) if (b.override) blocks[b.id] = b.override;

  return {
    md,
    meta: {
      id: s.id,
      page: s.page,
      blocks,
      outline: buildOutline(s.blocks),
      stats: sectionStats(s.blocks),
    },
  };
}

export function normalizeSection(section) {
  const seen = new Set();
  const blocks = (section?.blocks ?? []).map((b) => {
    let id = b.id || newBlockId();
    while (seen.has(id)) id = newBlockId();
    seen.add(id);
    return {
      id,
      md: String(b.md ?? ''),
      type: b.type ?? 'paragraph',
      override: normalizeOverride(b.override),
    };
  });
  return {
    id: section?.id || newSectionId(),
    name: section?.name ?? '섹션',
    page: { ...DEFAULT_PAGE, ...(section?.page ?? {}) },
    blocks: blocks.length ? blocks : [{ id: newBlockId(), md: '', type: 'paragraph', override: null }],
  };
}

/** Returns null when every field matches the default — keeps meta.json small. */
function normalizeOverride(o) {
  if (!o || typeof o !== 'object') return null;
  const out = {};
  if (o.align && o.align !== DEFAULT_OVERRIDE.align) out.align = o.align;
  if (Number(o.indent) > 0) out.indent = Number(o.indent);
  if (o.spacing && (o.spacing.before || o.spacing.after)) {
    out.spacing = {
      ...(o.spacing.before ? { before: Number(o.spacing.before) } : {}),
      ...(o.spacing.after ? { after: Number(o.spacing.after) } : {}),
    };
  }
  const style = {};
  for (const k of ['italic', 'bold', 'underline']) if (o.style?.[k]) style[k] = true;
  for (const k of ['color', 'bg', 'fontSize', 'font', 'lineHeight']) {
    if (o.style?.[k] !== undefined && o.style[k] !== '') style[k] = o.style[k];
  }
  if (Object.keys(style).length) out.style = style;
  return Object.keys(out).length ? out : null;
}

export function buildOutline(blocks) {
  const outline = [];
  for (const b of blocks) {
    const level = headingLevel(b.md);
    if (!level) continue;
    const text = plainText(b.md);
    if (text) outline.push({ level, text, anchor: b.id });
  }
  return outline;
}

export function sectionStats(blocks) {
  const text = blocks.map((b) => plainText(b.md)).join('\n');
  return {
    words: countWords(text),
    chars: text.replace(/\s/g, '').length,
    blocks: blocks.length,
  };
}

export function makeSection({ name = '새 섹션', heading = true } = {}) {
  const blocks = [];
  if (heading) blocks.push({ id: newBlockId(), md: `# ${name}`, type: 'heading', override: null });
  blocks.push({ id: newBlockId(), md: '내용을 입력하세요.', type: 'paragraph', override: null });
  return { id: newSectionId(), name, page: { ...DEFAULT_PAGE }, blocks };
}
