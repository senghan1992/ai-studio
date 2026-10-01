/**
 * Block resizing, the way PowerPoint does it.
 *
 * A plain drag is free. Shift locks the aspect ratio; Ctrl/Cmd (either one,
 * since Office teaches Ctrl and browsers teach Cmd) resizes from the centre
 * instead of the anchored edge. The two compose.
 *
 * `origin` is `{ x, y, w, h }` in canvas pixels, `mode` one of the eight
 * compass handles (`'se'`, `'n'` …), `dx`/`dy` the pointer travel in canvas
 * pixels. Only minimum sizes keep a block grabbable — placement is free, as
 * on the canvas.
 */

export const MIN_W = 48;
export const MIN_H = 28;

const roundBox = (box) => ({
  x: Math.round(box.x),
  y: Math.round(box.y),
  w: Math.round(box.w),
  h: Math.round(box.h),
});

export function resizeBox(origin, mode, dx, dy, opts = {}) {
  const { lockAspect = false, fromCenter = false } = opts;

  if (fromCenter) {
    return roundBox(resizeFromCenter(origin, mode, dx, dy, lockAspect));
  }

  let { x, y, w, h } = origin;
  if (mode.includes('e')) w = origin.w + dx;
  if (mode.includes('s')) h = origin.h + dy;
  if (mode.includes('w')) {
    w = origin.w - dx;
    x = origin.x + dx;
  }
  if (mode.includes('n')) {
    h = origin.h - dy;
    y = origin.y + dy;
  }

  if (lockAspect) {
    ({ x, y, w, h } = withLockedAspect(origin, mode, w, h));
  }

  // Keep the anchored edge fixed when the drag crosses it.
  if (w < MIN_W) {
    if (mode.includes('w') && !fromCenter) x = origin.x + origin.w - MIN_W;
    w = MIN_W;
  }
  if (h < MIN_H) {
    if (mode.includes('n') && !fromCenter) y = origin.y + origin.h - MIN_H;
    h = MIN_H;
  }
  return roundBox({ x, y, w, h });
}

/**
 * The same box with the origin's aspect ratio restored: the dragged
 * edge(s) lead and the rest follows. A corner drag takes its scale from
 * whichever axis moved further; a side handle sets its own axis and the
 * other is derived, centred on the origin's middle line.
 */
function withLockedAspect(origin, mode, w, h) {
  const ratio = origin.w / Math.max(1, origin.h);
  const horizontal = mode.includes('e') || mode.includes('w');
  const vertical = mode.includes('n') || mode.includes('s');
  let x = origin.x;
  let y = origin.y;

  if (horizontal && vertical) {
    const scale = Math.max(w / origin.w, h / origin.h);
    w = origin.w * scale;
    h = origin.h * scale;
  } else if (horizontal) {
    h = w / ratio;
    y = origin.y + (origin.h - h) / 2;
  } else if (vertical) {
    w = h * ratio;
    x = origin.x + (origin.w - w) / 2;
  }

  // Minimums, keeping the ratio: the anchored (opposite) edge stays put.
  if (w < MIN_W) {
    w = MIN_W;
    h = w / ratio;
  }
  if (h < MIN_H) {
    h = MIN_H;
    w = h * ratio;
  }
  if (mode.includes('w')) x = origin.x + origin.w - w;
  if (mode.includes('n')) y = origin.y + origin.h - h;
  return { x, y, w, h };
}

/** Growth mirrored about the origin's centre: the pointer moves both edges. */
function resizeFromCenter(origin, mode, dx, dy, lockAspect) {
  const cx = origin.x + origin.w / 2;
  const cy = origin.y + origin.h / 2;
  const east = mode.includes('e') ? dx : mode.includes('w') ? -dx : 0;
  const south = mode.includes('s') ? dy : mode.includes('n') ? -dy : 0;
  let w = origin.w + east * 2;
  let h = origin.h + south * 2;

  if (lockAspect) {
    const ratio = origin.w / Math.max(1, origin.h);
    const horizontal = mode.includes('e') || mode.includes('w');
    const vertical = mode.includes('n') || mode.includes('s');
    if (horizontal && vertical) {
      const scale = Math.max(w / origin.w, h / origin.h);
      w = origin.w * scale;
      h = origin.h * scale;
    } else if (horizontal) {
      h = w / ratio;
    } else if (vertical) {
      w = h * ratio;
    }
  }

  w = Math.max(MIN_W, w);
  h = Math.max(MIN_H, h);
  return { x: cx - w / 2, y: cy - h / 2, w, h };
}
