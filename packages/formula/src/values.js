/** A cell error value, e.g. #DIV/0!. Propagates through operators like Excel. */
export class ErrValue {
  constructor(code) {
    this.code = code || '#VALUE!';
  }
  toString() {
    return this.code;
  }
}

/** The result of evaluating a range reference. */
export class RangeValue {
  constructor(cells, rows, cols) {
    this.cells = cells; // [{ ref, value }]
    this.rows = rows;
    this.cols = cols;
  }
  values() {
    return this.cells.map((c) => c.value);
  }
  /** Row-major 2D array, for INDEX/VLOOKUP. */
  grid() {
    const out = [];
    for (let r = 0; r < this.rows; r++) out.push(this.cells.slice(r * this.cols, (r + 1) * this.cols).map((c) => c.value));
    return out;
  }
}

export const err = (code) => new ErrValue(code);
export const isErr = (v) => v instanceof ErrValue;
export const isRange = (v) => v instanceof RangeValue;
export const BLANK = null;

/** Flatten args, expanding ranges and arrays, dropping nothing. */
export function flatten(args) {
  const out = [];
  const walk = (v) => {
    if (isRange(v)) v.values().forEach(walk);
    else if (Array.isArray(v)) v.forEach(walk);
    else out.push(v);
  };
  args.forEach(walk);
  return out;
}

/** First error found in args, or null. */
export function firstError(args) {
  for (const v of flatten(args)) if (isErr(v)) return v;
  return null;
}

/** Excel-style numeric coercion. Blanks are 0; unparseable text is an error. */
export function toNumber(v) {
  if (v === null || v === undefined || v === '') return 0;
  if (typeof v === 'number') return Number.isFinite(v) ? v : err('#NUM!');
  if (typeof v === 'boolean') return v ? 1 : 0;
  if (isErr(v)) return v;
  if (isRange(v)) return toNumber(v.values()[0] ?? 0);
  const s = String(v).trim().replace(/,/g, '');
  if (s === '') return 0;
  const n = Number(s);
  if (Number.isFinite(n)) return n;
  const pct = s.match(/^(-?[\d.]+)%$/);
  if (pct) return Number(pct[1]) / 100;
  return err('#VALUE!');
}

export function toText(v) {
  if (v === null || v === undefined) return '';
  if (isErr(v)) return v.code;
  if (typeof v === 'boolean') return v ? 'TRUE' : 'FALSE';
  if (isRange(v)) return toText(v.values()[0]);
  if (typeof v === 'number') return formatPlainNumber(v);
  return String(v);
}

export function toBoolean(v) {
  if (isErr(v)) return v;
  if (typeof v === 'boolean') return v;
  if (v === null || v === undefined || v === '') return false;
  if (typeof v === 'number') return v !== 0;
  const s = String(v).trim().toUpperCase();
  if (s === 'TRUE') return true;
  if (s === 'FALSE') return false;
  const n = Number(s);
  if (Number.isFinite(n)) return n !== 0;
  return err('#VALUE!');
}

/** Numbers only, skipping text/blanks — the SUM/AVERAGE contract. */
export function numericValues(args) {
  const out = [];
  for (const v of flatten(args)) {
    if (isErr(v)) return v;
    if (v === null || v === undefined || v === '') continue;
    if (typeof v === 'boolean') continue;
    if (typeof v === 'number') {
      out.push(v);
      continue;
    }
    const n = Number(String(v).replace(/,/g, ''));
    if (Number.isFinite(n)) out.push(n);
  }
  return out;
}

/** Shortest round-trippable decimal, avoiding 0.30000000000000004 in output. */
export function formatPlainNumber(n) {
  if (!Number.isFinite(n)) return String(n);
  if (Number.isInteger(n)) return String(n);
  const rounded = Number(n.toPrecision(12));
  return String(rounded);
}

/** Excel comparison semantics: numbers < text, case-insensitive text. */
export function compareValues(a, b) {
  const an = typeof a === 'number' || typeof a === 'boolean';
  const bn = typeof b === 'number' || typeof b === 'boolean';
  if (a === null || a === '') return b === null || b === '' ? 0 : bn ? cmp(0, Number(b)) : -1;
  if (b === null || b === '') return an ? cmp(Number(a), 0) : 1;
  if (an && bn) return cmp(Number(a), Number(b));
  if (an) return -1;
  if (bn) return 1;
  return cmp(String(a).toLowerCase(), String(b).toLowerCase());
}

function cmp(a, b) {
  return a < b ? -1 : a > b ? 1 : 0;
}
