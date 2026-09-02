/**
 * Adjust handles: the yellow diamonds Office puts on a shape.
 *
 * A preset's geometry is parameterised by `adj` values, and the handle is how
 * you change one — a rounded rectangle's corner radius, an arrow's shaft
 * thickness, a callout's tail. Where the handle sits and what a drag there means
 * is specific to each preset, so each descriptor carries both directions:
 * `at` puts the handle somewhere for a value, `read` turns a pointer position
 * back into one.
 *
 * A preset with no entry shows no adjust handle, which is what Office does for a
 * shape that has nothing to adjust. An imported shape's values are kept and
 * drawn either way; this governs only whether you can drag them.
 */

/** Fractions of the box, clamped to it. */
const clamp01 = (v) => Math.min(Math.max(v, 0), 1);

/**
 * @param name  the OOXML handle: `adj`, `adj1`, `adj2`
 * @param def   the value Office defaults to
 * @param range the value at fraction 0 and at fraction 1
 * @param at    fraction -> `{ x, y }` in box fractions
 * @param read  a pointer at `{ x, y }` box fractions -> fraction
 */
const handle = (name, def, range, at, read) => ({ name, def, range, at, read });

/** A corner radius: the handle rides the top edge at half the fraction. */
const radius = (name = 'adj', def = 16667) =>
  handle(name, def, [0, 50000], (f) => ({ x: f / 2, y: 0 }), (p) => p.x * 2);

/** A frame or bevel thickness: the handle sits on the diagonal. */
const inset = (name, def) =>
  handle(name, def, [0, 50000], (f) => ({ x: f, y: f }), (p) => (p.x + p.y) / 2);

/** A fraction read straight off the horizontal position. */
const alongX = (name, def, hi = 100000, y = 0) =>
  handle(name, def, [0, hi], (f) => ({ x: f, y }), (p) => p.x);

/** The same, measured back from the right edge. */
const fromRight = (name, def, hi = 100000, y = 0.5) =>
  handle(name, def, [0, hi], (f) => ({ x: 1 - f, y }), (p) => 1 - p.x);

/** Half-thickness about the horizontal centre line. */
const halfHeight = (name, def, range = [5000, 100000]) =>
  handle(name, def, range, (f) => ({ x: 0, y: 0.5 - f / 2 }), (p) => (0.5 - p.y) * 2);

/** A block arrow: shaft thickness, then head length. */
const ARROW = [halfHeight('adj1', 50000), fromRight('adj2', 50000)];

/** A callout tail, which follows the pointer on both axes. */
const TAIL = [
  handle('adj1', -20000, [-150000, 150000], (f) => ({ x: 0.5 + f, y: 1 }), (p) => p.x - 0.5),
  handle('adj2', 80000, [-150000, 150000], (f) => ({ x: 0.5, y: 0.5 + f }), (p) => p.y - 0.5),
];

const ADJUST = {
  /* rectangles — corner treatment */
  roundRect: [radius()],
  round1Rect: [radius()],
  round2SameRect: [radius('adj1')],
  round2DiagRect: [radius('adj1')],
  snip1Rect: [radius()],
  snip2SameRect: [radius('adj1')],
  snip2DiagRect: [radius('adj1')],
  snipRoundRect: [radius('adj1')],
  plaque: [radius()],
  flowChartAlternateProcess: [radius()],
  foldedCorner: [
    handle('adj', 16667, [0, 50000], (f) => ({ x: 1 - f, y: 1 }), (p) => 1 - p.x),
  ],

  /* outlines and stripes */
  frame: [inset('adj1', 12500)],
  bevel: [inset('adj1', 12500)],
  halfFrame: [inset('adj1', 20000)],
  corner: [alongX('adj1', 50000)],
  diagStripe: [alongX('adj', 50000)],

  /* an apex or a slant */
  triangle: [alongX('adj', 50000)],
  parallelogram: [alongX('adj', 25000)],
  trapezoid: [alongX('adj', 25000, 50000)],
  flowChartManualOperation: [alongX('adj', 25000, 50000)],
  homePlate: [fromRight('adj', 25000)],
  chevron: [fromRight('adj', 25000)],

  /* block arrows */
  rightArrow: ARROW,
  leftArrow: ARROW,
  upArrow: ARROW,
  downArrow: ARROW,
  leftRightArrow: ARROW,
  upDownArrow: ARROW,

  /* rings and crosses */
  donut: [handle('adj', 25000, [0, 90000], (f) => ({ x: 0.5, y: f / 2 }), (p) => p.y * 2)],
  plus: [inset('adj', 25000)],
  mathPlus: [inset('adj', 25000)],
  mathMinus: [halfHeight('adj1', 23077, [0, 50000])],

  /* callouts */
  wedgeRectCallout: TAIL,
  wedgeRoundRectCallout: TAIL,
  wedgeEllipseCallout: TAIL,
};

/** The adjust handles a shape offers, with where each one currently sits. */
export function adjustHandles(shape) {
  const descriptors = ADJUST[shape?.preset];
  if (!Array.isArray(descriptors)) return [];

  return descriptors.map((d) => {
    const raw = shape?.adjust?.[d.name];
    const value = Number.isFinite(raw) ? raw : d.def;
    const fraction = toFraction(value, d.range);
    return { name: d.name, value, at: d.at(fraction), read: d.read, range: d.range };
  });
}

/** The value a drag to `{ x, y }` — fractions of the box — should write. */
export function adjustValueAt(handleAt, point) {
  const fraction = clamp01(handleAt.read(point));
  const [lo, hi] = handleAt.range;
  return Math.round(lo + fraction * (hi - lo));
}

/** True when this preset has anything to adjust. */
export function hasAdjust(preset) {
  return Array.isArray(ADJUST[preset]) && ADJUST[preset].length > 0;
}

function toFraction(value, [lo, hi]) {
  if (hi === lo) return 0;
  return clamp01((value - lo) / (hi - lo));
}
