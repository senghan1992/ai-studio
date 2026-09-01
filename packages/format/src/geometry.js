export const DEFAULT_CANVAS = { w: 1280, h: 720, bg: '#ffffff' };

const V_BANDS = [
  [0.28, '상단'],
  [0.68, '중앙'],
  [Infinity, '하단'],
];
const H_BANDS = [
  [0.3, '좌측'],
  [0.7, '중앙'],
  [Infinity, '우측'],
];

function band(ratio, bands) {
  for (const [limit, label] of bands) if (ratio < limit) return label;
  return bands[bands.length - 1][1];
}

/**
 * Translate pixel geometry into a phrase an LLM can reason about.
 *
 * This is the single most useful thing the digest does: models handle
 * "상단 중앙, 슬라이드 폭의 85%" far better than "x=96 y=220 w=1088".
 */
export function positionPhrase(box, canvas = DEFAULT_CANVAS) {
  if (!box) return '';
  const cx = (box.x + box.w / 2) / canvas.w;
  const cy = (box.y + box.h / 2) / canvas.h;
  const v = band(cy, V_BANDS);
  const h = band(cx, H_BANDS);
  const where = v === '중앙' && h === '중앙' ? '정중앙' : `${v} ${h}`;

  const widthPct = Math.round((box.w / canvas.w) * 100);
  const size = widthPct >= 85 ? '전체 폭' : widthPct <= 35 ? '좁은 폭' : `폭 ${widthPct}%`;
  return `${where}, ${size}`;
}

/** Reading order: top-to-bottom, then left-to-right, with a row tolerance. */
export function readingOrder(blocks, rowTolerance = 40) {
  return [...blocks].sort((a, b) => {
    const dy = (a.y ?? 0) - (b.y ?? 0);
    if (Math.abs(dy) > rowTolerance) return dy;
    return (a.x ?? 0) - (b.x ?? 0);
  });
}

/**
 * Vertical stack fallback for blocks with no saved geometry, so a markdown-only
 * slide still renders sensibly instead of piling everything at the origin.
 */
export function autoLayout(index, total, canvas = DEFAULT_CANVAS) {
  const margin = Math.round(canvas.w * 0.075);
  const top = Math.round(canvas.h * 0.14);
  const gap = 24;
  const usable = canvas.h - top - margin;
  const h = Math.max(64, Math.round((usable - gap * Math.max(0, total - 1)) / Math.max(1, total)));
  return {
    x: margin,
    y: top + index * (h + gap),
    w: canvas.w - margin * 2,
    h,
    z: index + 1,
  };
}

/** Clamp a box inside the canvas but never below a usable minimum size. */
export function clampBox(box, canvas = DEFAULT_CANVAS) {
  const w = Math.min(Math.max(40, Math.round(box.w)), canvas.w);
  const h = Math.min(Math.max(28, Math.round(box.h)), canvas.h);
  return {
    ...box,
    w,
    h,
    x: Math.min(Math.max(0, Math.round(box.x)), canvas.w - w),
    y: Math.min(Math.max(0, Math.round(box.y)), canvas.h - h),
  };
}
