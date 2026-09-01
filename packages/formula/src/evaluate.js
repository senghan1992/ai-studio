import { parse, collectRefs, FormulaError } from './parse.js';
import { FUNCTIONS, applyNumFmt, toSerial } from './functions.js';
import {
  ErrValue, RangeValue, err, isErr, isRange,
  toNumber, toText, toBoolean, compareValues, formatPlainNumber,
} from './values.js';
import { parseRange, expandRange, rangeSize, parseRef } from './refs.js';

/**
 * Evaluate one AST against a context.
 * @param {object} ast
 * @param {{ getValue(ref): any, getRange?(range): RangeValue, names?: object }} ctx
 */
export function evaluate(ast, ctx) {
  const getRange = ctx.getRange ?? ((range) => defaultGetRange(range, ctx));

  const walk = (node) => {
    switch (node.kind) {
      case 'number':
      case 'string':
      case 'boolean':
        return node.value;
      case 'blank':
        return null;
      case 'error':
        return err(node.value);
      case 'ref':
        // Strip $ anchors here rather than in every getValue implementation:
        // $E$7 and E7 are the same cell, and only rewriting cares about anchors.
        return ctx.getValue(bareRef(node.ref));
      case 'range':
        return getRange(node.range);
      case 'array':
        return node.items.map(walk);
      case 'name': {
        const target = ctx.names?.[node.name] ?? ctx.names?.[node.name.toUpperCase()];
        if (target === undefined) return err('#NAME?');
        if (typeof target === 'string' && parseRange(target)) return getRange(target);
        if (typeof target === 'string' && parseRef(target)) return ctx.getValue(bareRef(target));
        return target;
      }
      case 'percent': {
        const v = toNumber(walk(node.arg));
        return isErr(v) ? v : v / 100;
      }
      case 'unary': {
        const v = toNumber(walk(node.arg));
        if (isErr(v)) return v;
        return node.op === '-' ? -v : v;
      }
      case 'binary':
        return binary(node.op, walk(node.left), walk(node.right));
      case 'call': {
        const fn = FUNCTIONS[node.name];
        if (!fn) return err('#NAME?');
        // IF/IFERROR need unevaluated laziness only for perf, not correctness here.
        const args = node.args.map(walk);
        try {
          const out = fn(args, ctx);
          return out === undefined ? null : out;
        } catch (e) {
          return err(e instanceof FormulaError ? e.code : '#VALUE!');
        }
      }
      default:
        return err('#VALUE!');
    }
  };

  return walk(ast);
}

/** `$E$7` -> `E7`. The stored key form for a cell. */
export function bareRef(ref) {
  return String(ref).replace(/\$/g, '').toUpperCase();
}

function defaultGetRange(range, ctx) {
  const r = parseRange(range);
  if (!r) return err('#REF!');
  const { rows, cols } = rangeSize(r);
  const cells = expandRange(r).map((ref) => ({ ref, value: ctx.getValue(ref) }));
  return new RangeValue(cells, rows, cols);
}

function binary(op, left, right) {
  const l = scalar(left);
  const r = scalar(right);
  if (isErr(l)) return l;
  if (isErr(r)) return r;

  if (op === '&') return toText(l) + toText(r);

  if (['=', '<>', '<', '>', '<=', '>='].includes(op)) {
    const c = compareValues(l, r);
    switch (op) {
      case '=': return c === 0;
      case '<>': return c !== 0;
      case '<': return c < 0;
      case '>': return c > 0;
      case '<=': return c <= 0;
      default: return c >= 0;
    }
  }

  const a = toNumber(l);
  const b = toNumber(r);
  if (isErr(a)) return a;
  if (isErr(b)) return b;

  switch (op) {
    case '+': return a + b;
    case '-': return a - b;
    case '*': return a * b;
    case '/': return b === 0 ? err('#DIV/0!') : a / b;
    case '^': {
      const p = a ** b;
      return Number.isFinite(p) ? p : err('#NUM!');
    }
    default: return err('#VALUE!');
  }
}

/** Ranges collapse to their first cell in scalar position. */
function scalar(v) {
  if (isRange(v)) {
    const first = v.values()[0];
    return first === undefined ? null : first;
  }
  if (Array.isArray(v)) return v[0] ?? null;
  return v;
}

/* --------------------------------------------------------- sheet recalc */

const MAX_ITER_GUARD = 200000;

/**
 * Recalculate every formula in a sheet.
 *
 * Formula cells are evaluated on demand with memoisation; a cell already on the
 * evaluation stack yields #CIRC! instead of blowing the JS stack. Literal cells
 * are returned untouched, so a sheet with no formulas costs one pass.
 *
 * @param {{cells: object, names?: object}} sheet
 * @returns {{cells: object, changed: string[], errors: object}}
 */
export function recalcSheet(sheet) {
  const source = sheet?.cells ?? {};
  const names = sheet?.names ?? {};
  const out = {};
  const memo = new Map();
  const visiting = new Set();
  const errors = {};
  const changed = [];
  let steps = 0;

  const astCache = new Map();
  const astFor = (ref, formula) => {
    if (astCache.has(ref)) return astCache.get(ref);
    let ast;
    try {
      ast = parse(formula);
    } catch (e) {
      ast = { kind: 'error', value: e instanceof FormulaError ? e.code : '#VALUE!' };
    }
    astCache.set(ref, ast);
    return ast;
  };

  const valueOf = (ref) => {
    const key = bareRef(ref);
    if (memo.has(key)) return memo.get(key);
    if (++steps > MAX_ITER_GUARD) return err('#NUM!');

    const cell = source[key];
    if (!cell) return null;

    if (typeof cell.f === 'string' && cell.f.trim().startsWith('=')) {
      if (visiting.has(key)) return err('#CIRC!');
      visiting.add(key);
      let value;
      try {
        value = evaluate(astFor(key, cell.f), { getValue: valueOf, names });
      } catch (e) {
        value = err(e instanceof FormulaError ? e.code : '#VALUE!');
      } finally {
        visiting.delete(key);
      }
      if (isRange(value)) value = value.values()[0] ?? null;
      memo.set(key, value);
      return value;
    }

    const literal = cell.v ?? null;
    memo.set(key, literal);
    return literal;
  };

  for (const [ref, cell] of Object.entries(source)) {
    const key = ref.toUpperCase();
    if (typeof cell.f === 'string' && cell.f.trim().startsWith('=')) {
      const value = valueOf(key);
      const next = { ...cell, ...typed(value) };
      if (next.v !== cell.v || next.t !== cell.t) changed.push(key);
      if (isErr(value)) errors[key] = value.code;
      out[key] = next;
    } else {
      out[key] = { ...cell, ...typed(cell.v ?? null) };
    }
  }

  return { cells: out, changed, errors };
}

/** Convert an evaluated value into the stored `{v, t}` pair. */
export function typed(value) {
  if (isErr(value)) return { v: value.code, t: 'e' };
  if (value === null || value === undefined || value === '') return { v: null, t: 'z' };
  if (typeof value === 'number') return { v: Number(Number(value).toPrecision(14)), t: 'n' };
  if (typeof value === 'boolean') return { v: value, t: 'b' };
  return { v: String(value), t: 's' };
}

/**
 * Interpret what a user typed into a cell.
 * Returns a cell object ready to store: formula, number, boolean, date or text.
 */
export function parseCellInput(raw) {
  const input = raw === null || raw === undefined ? '' : String(raw);
  const trimmed = input.trim();

  if (trimmed === '') return null; // clears the cell
  if (trimmed.startsWith('=')) return { f: trimmed, v: null, t: 'n' };

  const upper = trimmed.toUpperCase();
  if (upper === 'TRUE' || upper === 'FALSE') return { v: upper === 'TRUE', t: 'b' };

  const percent = trimmed.match(/^(-?[\d,]*\.?\d+)\s*%$/);
  if (percent) {
    return { v: Number(percent[1].replace(/,/g, '')) / 100, t: 'n', fmt: '0.0%' };
  }

  const currency = trimmed.match(/^([₩$€£¥])\s*(-?[\d,]*\.?\d+)$/);
  if (currency) {
    return { v: Number(currency[2].replace(/,/g, '')), t: 'n', fmt: `${currency[1]}#,##0` };
  }

  const numeric = trimmed.replace(/,/g, '');
  if (/^-?(\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/.test(numeric)) {
    return { v: Number(numeric), t: 'n', ...(trimmed.includes(',') ? { fmt: '#,##0' } : {}) };
  }

  const isoDate = trimmed.match(/^(\d{4})[-/](\d{1,2})[-/](\d{1,2})$/);
  if (isoDate) {
    const d = new Date(Date.UTC(Number(isoDate[1]), Number(isoDate[2]) - 1, Number(isoDate[3])));
    if (!Number.isNaN(d.getTime())) return { v: toSerial(d), t: 'd', fmt: 'yyyy-mm-dd' };
  }

  return { v: input, t: 's' };
}

/** What the user sees in a cell: formatted value, or the error code. */
export function displayValue(cell) {
  if (!cell) return '';
  if (cell.t === 'e') return String(cell.v ?? '#VALUE!');
  if (cell.v === null || cell.v === undefined) return '';
  if (cell.fmt) return applyNumFmt(cell.v, cell.fmt);
  if (typeof cell.v === 'number') return formatPlainNumber(cell.v);
  if (typeof cell.v === 'boolean') return cell.v ? 'TRUE' : 'FALSE';
  return String(cell.v);
}

/** What the user edits: the formula if there is one, else the raw value. */
export function editValue(cell) {
  if (!cell) return '';
  if (cell.f) return cell.f;
  if (cell.v === null || cell.v === undefined) return '';
  if (cell.t === 'd' && cell.fmt) return applyNumFmt(cell.v, cell.fmt);
  if (typeof cell.v === 'number') return formatPlainNumber(cell.v);
  if (typeof cell.v === 'boolean') return cell.v ? 'TRUE' : 'FALSE';
  return String(cell.v);
}

/** Direct dependencies of a formula, for the editor's dependency highlight. */
export function dependencies(formula) {
  try {
    const refs = collectRefs(parse(formula));
    const cells = new Set(refs.refs.map((r) => r.replace(/\$/g, '').toUpperCase()));
    for (const range of refs.ranges) expandRange(range).forEach((r) => cells.add(r));
    return [...cells];
  } catch {
    return [];
  }
}
