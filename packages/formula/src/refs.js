const A = 'A'.charCodeAt(0);

/** 'A' -> 0, 'Z' -> 25, 'AA' -> 26 */
export function colToIndex(col) {
  let n = 0;
  const s = String(col).toUpperCase();
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i) - A;
    if (c < 0 || c > 25) return -1;
    n = n * 26 + c + 1;
  }
  return n - 1;
}

/** 0 -> 'A', 26 -> 'AA' */
export function indexToCol(index) {
  let n = Math.max(0, Math.floor(index)) + 1;
  let out = '';
  while (n > 0) {
    const rem = (n - 1) % 26;
    out = String.fromCharCode(A + rem) + out;
    n = Math.floor((n - 1) / 26);
  }
  return out;
}

const REF_RE = /^\$?([A-Za-z]{1,3})\$?([1-9]\d{0,6})$/;

/** 'B12' -> { col: 1, row: 11 } (both zero-based). null if not a ref. */
export function parseRef(ref) {
  const m = String(ref ?? '').trim().match(REF_RE);
  if (!m) return null;
  const col = colToIndex(m[1]);
  if (col < 0) return null;
  return { col, row: Number(m[2]) - 1 };
}

/** { col, row } -> 'B12' */
export function toRef(col, row) {
  return `${indexToCol(col)}${row + 1}`;
}

/** 'A1:B3' -> { start, end } normalized so start is top-left. null if invalid. */
export function parseRange(range) {
  const parts = String(range ?? '').split(':');
  if (parts.length !== 2) return null;
  const a = parseRef(parts[0]);
  const b = parseRef(parts[1]);
  if (!a || !b) return null;
  return {
    start: { col: Math.min(a.col, b.col), row: Math.min(a.row, b.row) },
    end: { col: Math.max(a.col, b.col), row: Math.max(a.row, b.row) },
  };
}

export function rangeToString(r) {
  return `${toRef(r.start.col, r.start.row)}:${toRef(r.end.col, r.end.row)}`;
}

/** Every cell address in a range, row-major. */
export function expandRange(range) {
  const r = typeof range === 'string' ? parseRange(range) : range;
  if (!r) return [];
  const out = [];
  for (let row = r.start.row; row <= r.end.row; row++) {
    for (let col = r.start.col; col <= r.end.col; col++) out.push(toRef(col, row));
  }
  return out;
}

export function rangeSize(range) {
  const r = typeof range === 'string' ? parseRange(range) : range;
  if (!r) return { rows: 0, cols: 0 };
  return { rows: r.end.row - r.start.row + 1, cols: r.end.col - r.start.col + 1 };
}

/** Shift a reference by (dc, dr), respecting $ anchors. Returns '#REF!' if off-sheet. */
export function shiftRef(ref, dc, dr) {
  const m = String(ref).match(/^(\$?)([A-Za-z]{1,3})(\$?)([1-9]\d{0,6})$/);
  if (!m) return ref;
  const [, colAbs, colStr, rowAbs, rowStr] = m;
  const col = colAbs ? colToIndex(colStr) : colToIndex(colStr) + dc;
  const row = rowAbs ? Number(rowStr) - 1 : Number(rowStr) - 1 + dr;
  if (col < 0 || row < 0) return '#REF!';
  return `${colAbs}${indexToCol(col)}${rowAbs}${row + 1}`;
}

const REF_TOKEN = /(\$?)([A-Za-z]{1,3})(\$?)([1-9]\d{0,6})/g;

/**
 * Rewrite the references in a formula after rows or columns are inserted or
 * removed, so `=SUM(B2:B4)` becomes `=SUM(B3:B5)` when a row is inserted above.
 *
 * A reference that pointed into deleted space becomes #REF!, matching Excel.
 * String literals are skipped so `="A1"` is left alone.
 */
export function adjustRefs(formula, axis, at, delta) {
  const src = String(formula ?? '');
  if (!src.startsWith('=') || delta === 0) return src;
  return mapOutsideStrings(src, (segment) => rewriteSegment(segment, axis, at, delta));
}

/**
 * Apply `fn` to the parts of a formula that are not inside a string literal, so
 * `="A1"` is never rewritten as a reference.
 */
function mapOutsideStrings(src, fn) {
  let out = '';
  let i = 0;
  while (i < src.length) {
    const quote = src.indexOf('"', i);
    const end = quote === -1 ? src.length : quote;
    out += fn(src.slice(i, end));
    if (quote === -1) break;
    const close = findClosingQuote(src, quote);
    out += src.slice(quote, close + 1);
    i = close + 1;
  }
  return out;
}

/**
 * Translate every relative reference in a formula by (dc, dr).
 *
 * This is what makes copy/paste and the fill handle behave: dragging `=B2+C2`
 * down one row must produce `=B3+C3`, while `$B$2` stays put.
 */
export function shiftFormula(formula, dc, dr) {
  const src = String(formula ?? '');
  if (!src.startsWith('=') || (dc === 0 && dr === 0)) return src;
  return mapOutsideStrings(src, (segment) =>
    segment.replace(REF_TOKEN, (match) => shiftRef(match, dc, dr))
  );
}

function findClosingQuote(src, start) {
  let i = start + 1;
  while (i < src.length) {
    if (src[i] === '"') {
      if (src[i + 1] === '"') {
        i += 2;
        continue;
      }
      return i;
    }
    i++;
  }
  return src.length - 1;
}

function rewriteSegment(segment, axis, at, delta) {
  return segment.replace(REF_TOKEN, (match, colAbs, colStr, rowAbs, rowStr) => {
    const col = colToIndex(colStr);
    const row = Number(rowStr) - 1;
    if (col < 0) return match;

    const index = axis === 'row' ? row : col;
    let next = index;

    if (delta > 0) {
      if (index >= at) next = index + delta;
    } else if (index >= at && index < at - delta) {
      return '#REF!';
    } else if (index >= at - delta) {
      next = index + delta;
    }

    if (next < 0) return '#REF!';
    const nextCol = axis === 'col' ? next : col;
    const nextRow = axis === 'row' ? next : row;
    return `${colAbs}${indexToCol(nextCol)}${rowAbs}${nextRow + 1}`;
  });
}
