/**
 * Align and distribute slide blocks.
 *
 * PowerPoint's rule, which is the one people expect: with several objects
 * selected they align to each other, with one they align to the slide. This editor
 * has single selection, so alignment is relative to the canvas — and distribution
 * always works across every block on the slide, which is the only sensible reading
 * with one selection.
 */

const EDGES = {
  left: (box, canvas) => ({ x: 0 }),
  right: (box, canvas) => ({ x: canvas.w - box.w }),
  hcenter: (box, canvas) => ({ x: Math.round((canvas.w - box.w) / 2) }),
  top: () => ({ y: 0 }),
  bottom: (box, canvas) => ({ y: canvas.h - box.h }),
  vcenter: (box, canvas) => ({ y: Math.round((canvas.h - box.h) / 2) }),
};

export function alignBlocks(blocks, edge, canvas, selectedId) {
  const move = EDGES[edge];
  if (!move) return blocks;
  return blocks.map((block) => {
    if (block.id !== selectedId || block.locked) return block;
    return { ...block, ...move(block, canvas) };
  });
}

/**
 * Even out the gaps between blocks along one axis.
 *
 * The outermost two stay put — they define the span — and the rest are spread so
 * the gaps between edges are equal, which is what "distribute" means in Office
 * (equal spacing, not equal centres).
 */
export function distributeBlocks(blocks, axis) {
  const movable = blocks.filter((b) => !b.locked);
  if (movable.length < 3) return blocks;

  const vertical = axis === 'vertical';
  const pos = (b) => (vertical ? b.y : b.x);
  const size = (b) => (vertical ? b.h : b.w);

  const sorted = [...movable].sort((a, b) => pos(a) - pos(b));
  const first = sorted[0];
  const last = sorted[sorted.length - 1];

  const span = pos(last) + size(last) - pos(first);
  const used = sorted.reduce((total, b) => total + size(b), 0);
  const gap = (span - used) / (sorted.length - 1);

  const placement = new Map();
  let cursor = pos(first);
  for (const block of sorted) {
    placement.set(block.id, Math.round(cursor));
    cursor += size(block) + gap;
  }

  return blocks.map((block) => {
    if (!placement.has(block.id)) return block;
    const value = placement.get(block.id);
    return vertical ? { ...block, y: value } : { ...block, x: value };
  });
}
