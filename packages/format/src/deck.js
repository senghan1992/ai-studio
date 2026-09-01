import { parseFrontmatter, serializeFrontmatter } from './frontmatter.js';
import { splitBlocks, joinBlocks, inferKind, blockLabel } from './blocks.js';
import { DEFAULT_CANVAS, autoLayout, clampBox } from './geometry.js';
import { newSlideId, newBlockId } from './ids.js';

export const SLIDE_LAYOUTS = ['title', 'title-content', 'two-column', 'section', 'blank'];

const DEFAULT_TEXT_STYLE = {
  fontSize: 20,
  weight: 400,
  align: 'left',
  valign: 'top',
  color: '#1f2937',
  lineHeight: 1.45,
};

/**
 * Merge a slide's markdown with its layout JSON into one editable object.
 *
 * Either input may be missing or stale: markdown is authoritative for *which*
 * blocks exist and what they say, JSON is authoritative for where they sit.
 * Blocks present only in JSON are dropped; blocks present only in markdown get
 * an auto layout.
 */
export function readSlide({ md = '', layout = null } = {}) {
  const { meta, body } = parseFrontmatter(md);
  const parsed = splitBlocks(body);
  const geo = layout?.blocks ?? {};
  const canvas = { ...DEFAULT_CANVAS, ...(layout?.canvas ?? {}) };

  const blocks = parsed.map((b, i) => {
    const g = geo[b.id] ?? {};
    const kind = g.kind ?? b.hint.kind ?? inferKind(b.md);
    const fallback = autoLayout(i, parsed.length, canvas);
    const box = {
      x: num(g.x, fallback.x),
      y: num(g.y, fallback.y),
      w: num(g.w, fallback.w),
      h: num(g.h, fallback.h),
      z: num(g.z, fallback.z),
    };
    return {
      id: b.id,
      kind,
      md: b.md,
      ...clampBox(box, canvas),
      style: kind === 'text' ? { ...DEFAULT_TEXT_STYLE, ...(g.style ?? {}) } : { ...(g.style ?? {}) },
      locked: !!g.locked,
    };
  });

  return {
    id: meta.id || layout?.id || newSlideId(),
    title: meta.title ?? deriveTitle(blocks),
    layoutName: SLIDE_LAYOUTS.includes(meta.layout) ? meta.layout : 'title-content',
    notes: meta.notes ?? '',
    canvas,
    blocks,
  };
}

/** Split a slide back into the markdown + layout JSON pair written to disk. */
export function writeSlide(slide) {
  const s = normalizeSlide(slide);
  const md = serializeFrontmatter(
    { id: s.id, title: s.title, layout: s.layoutName, notes: s.notes || undefined },
    joinBlocks(s.blocks)
  );
  const blocks = {};
  for (const b of s.blocks) {
    blocks[b.id] = {
      x: b.x,
      y: b.y,
      w: b.w,
      h: b.h,
      z: b.z,
      kind: b.kind,
      ...(b.locked ? { locked: true } : {}),
      ...(Object.keys(b.style ?? {}).length ? { style: b.style } : {}),
    };
  }
  return {
    md,
    layout: { id: s.id, canvas: s.canvas, blocks },
  };
}

export function normalizeSlide(slide) {
  const canvas = { ...DEFAULT_CANVAS, ...(slide?.canvas ?? {}) };
  const seen = new Set();
  const blocks = (slide?.blocks ?? []).map((b, i) => {
    let id = b.id || newBlockId();
    while (seen.has(id)) id = newBlockId();
    seen.add(id);
    const kind = b.kind ?? inferKind(b.md);
    const box = clampBox(
      {
        x: num(b.x, autoLayout(i, 1, canvas).x),
        y: num(b.y, autoLayout(i, 1, canvas).y),
        w: num(b.w, 400),
        h: num(b.h, 120),
      },
      canvas
    );
    return {
      id,
      kind,
      md: String(b.md ?? ''),
      ...box,
      z: num(b.z, i + 1),
      style: b.style ?? {},
      locked: !!b.locked,
    };
  });
  return {
    id: slide?.id || newSlideId(),
    title: slide?.title ?? deriveTitle(blocks),
    layoutName: SLIDE_LAYOUTS.includes(slide?.layoutName) ? slide.layoutName : 'title-content',
    notes: slide?.notes ?? '',
    canvas,
    blocks,
  };
}

function deriveTitle(blocks) {
  for (const b of blocks) {
    const label = blockLabel(b.md);
    if (label) return label;
  }
  return '제목 없는 슬라이드';
}

function num(v, fallback) {
  return Number.isFinite(Number(v)) ? Number(v) : fallback;
}

/** Blank slide for a chosen semantic layout, pre-populated like PowerPoint does. */
export function makeSlide(layoutName = 'title-content', { title, index = 1 } = {}) {
  const canvas = { ...DEFAULT_CANVAS };
  const id = newSlideId();
  const text = (md, box, style) => ({
    id: newBlockId(),
    kind: 'text',
    md,
    ...box,
    style: { ...DEFAULT_TEXT_STYLE, ...style },
  });

  let blocks;
  switch (layoutName) {
    case 'title':
      blocks = [
        text(`# ${title ?? '프레젠테이션 제목'}`, { x: 96, y: 260, w: 1088, h: 120, z: 1 }, { fontSize: 54, weight: 700, align: 'center' }),
        text('부제목을 입력하세요', { x: 96, y: 396, w: 1088, h: 60, z: 2 }, { fontSize: 22, align: 'center', color: '#6b7280' }),
      ];
      break;
    case 'section':
      blocks = [
        text(`## ${title ?? '섹션'}`, { x: 96, y: 300, w: 1088, h: 100, z: 1 }, { fontSize: 40, weight: 600, align: 'center' }),
      ];
      break;
    case 'two-column':
      blocks = [
        text(`# ${title ?? `슬라이드 ${index}`}`, { x: 96, y: 72, w: 1088, h: 80, z: 1 }, { fontSize: 36, weight: 700 }),
        text('- 왼쪽 항목', { x: 96, y: 196, w: 512, h: 420, z: 2 }),
        text('- 오른쪽 항목', { x: 672, y: 196, w: 512, h: 420, z: 3 }),
      ];
      break;
    case 'blank':
      blocks = [];
      break;
    default:
      blocks = [
        text(`# ${title ?? `슬라이드 ${index}`}`, { x: 96, y: 72, w: 1088, h: 80, z: 1 }, { fontSize: 36, weight: 700 }),
        text('- 첫 번째 항목\n- 두 번째 항목', { x: 96, y: 196, w: 1088, h: 420, z: 2 }),
      ];
  }

  return { id, title: title ?? `슬라이드 ${index}`, layoutName, notes: '', canvas, blocks };
}
