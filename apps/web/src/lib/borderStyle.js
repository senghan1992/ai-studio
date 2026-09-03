/**
 * One cell edge's CSS border, from the value stored in `style.border`.
 *
 * An edge is `true` for the plain thin line the editor draws, or an object
 * `{ style, color }` carried over from an imported workbook — Excel's line
 * names (`dotted`, `medium`, `double`, …) and an `#rrggbb` colour. Rendering
 * both from one place keeps the grid, the print view and the export in step.
 */

const DEFAULT_COLOR = '#9ca3af';

const WIDTH = {
  hair: '1px',
  thin: '1px',
  dotted: '1px',
  dashed: '1px',
  dashDot: '1px',
  dashDotDot: '1px',
  medium: '2px',
  mediumDashed: '2px',
  mediumDashDot: '2px',
  mediumDashDotDot: '2px',
  slantDashDot: '2px',
  thick: '3px',
  double: '3px',
};

const LINE = {
  dotted: 'dotted',
  dashed: 'dashed',
  dashDot: 'dashed',
  dashDotDot: 'dashed',
  mediumDashed: 'dashed',
  mediumDashDot: 'dashed',
  mediumDashDotDot: 'dashed',
  slantDashDot: 'dashed',
  double: 'double',
};

export function borderCss(edge) {
  if (!edge) return undefined;
  if (edge === true) return `1px solid ${DEFAULT_COLOR}`;
  if (typeof edge !== 'object') return undefined;
  if (edge.style === 'none') return undefined;
  const style = edge.style && WIDTH[edge.style] ? edge.style : 'thin';
  const color = typeof edge.color === 'string' && edge.color ? edge.color : DEFAULT_COLOR;
  return `${WIDTH[style]} ${LINE[style] ?? 'solid'} ${color}`;
}

/** The four CSS border properties for a `style.border` value, only the set ones. */
export function borderStyles(border) {
  const out = {};
  if (!border) return out;
  const t = borderCss(border.t);
  const b = borderCss(border.b);
  const l = borderCss(border.l);
  const r = borderCss(border.r);
  if (t) out.borderTop = t;
  if (b) out.borderBottom = b;
  if (l) out.borderLeft = l;
  if (r) out.borderRight = r;
  return out;
}
