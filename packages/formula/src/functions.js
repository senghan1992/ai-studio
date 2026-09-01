import {
  ErrValue, RangeValue, err, isErr, isRange, flatten, firstError,
  toNumber, toText, toBoolean, numericValues, compareValues,
} from './values.js';

const E = {
  VALUE: () => err('#VALUE!'),
  NUM: () => err('#NUM!'),
  DIV0: () => err('#DIV/0!'),
  NA: () => err('#N/A'),
  REF: () => err('#REF!'),
};

/** Unwrap a numeric arg, short-circuiting on error. */
function n(v) {
  const x = toNumber(v);
  return x;
}

function guard(fn) {
  return (args, ctx) => {
    const e = firstError(args);
    if (e) return e;
    return fn(args, ctx);
  };
}

/* --------------------------------------------------------- criteria match */

/**
 * Parse a SUMIF/COUNTIF criterion: 42, ">10", "<>x", "seoul", "a*".
 * Returns a predicate over cell values.
 */
export function makeCriteria(raw) {
  const value = isRange(raw) ? raw.values()[0] : raw;
  if (typeof value === 'number' || typeof value === 'boolean') {
    return (v) => compareValues(v, value) === 0;
  }
  const s = toText(value);
  const m = s.match(/^(<=|>=|<>|<|>|=)\s*(.*)$/);
  if (m) {
    const [, op, rest] = m;
    const target = rest === '' ? null : Number.isFinite(Number(rest)) ? Number(rest) : rest;
    return (v) => {
      const c = compareValues(v, target);
      switch (op) {
        case '>': return c > 0;
        case '<': return c < 0;
        case '>=': return c >= 0;
        case '<=': return c <= 0;
        case '<>': return c !== 0;
        default: return c === 0;
      }
    };
  }
  if (/[*?]/.test(s)) {
    const rx = new RegExp(`^${s.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.')}$`, 'i');
    return (v) => rx.test(toText(v));
  }
  const target = Number.isFinite(Number(s)) && s.trim() !== '' ? Number(s) : s;
  return (v) => compareValues(v, target) === 0;
}

function pairedIf(rangeArg, criteriaArg, sumArg) {
  const range = isRange(rangeArg) ? rangeArg : null;
  if (!range) return E.VALUE();
  const test = makeCriteria(criteriaArg);
  const target = isRange(sumArg) ? sumArg : range;
  const picked = [];
  range.values().forEach((v, i) => {
    if (test(v)) picked.push(target.values()[i]);
  });
  return picked;
}

/* --------------------------------------------------------------- lookups */

function lookupIn(needle, haystack, resultIndex, approximate, horizontal) {
  if (!isRange(haystack)) return E.VALUE();
  const grid = haystack.grid();
  const idx = Math.trunc(toNumber(resultIndex));
  if (isErr(idx)) return idx;

  const lines = horizontal ? grid : grid.map((row) => row); // rows
  const keys = horizontal ? grid[0] ?? [] : grid.map((r) => r[0]);
  if (idx < 1 || idx > (horizontal ? grid.length : (grid[0] ?? []).length)) return E.REF();

  let found = -1;
  if (approximate) {
    for (let i = 0; i < keys.length; i++) {
      const c = compareValues(keys[i], needle);
      if (c <= 0) found = i;
      else break;
    }
  } else {
    const test = makeCriteria(needle);
    found = keys.findIndex((k) => test(k));
  }
  if (found < 0) return E.NA();
  const value = horizontal ? grid[idx - 1]?.[found] : grid[found]?.[idx - 1];
  return value ?? null;
}

/* ------------------------------------------------------------ date serial */

const DAY_MS = 86400000;
const EPOCH = Date.UTC(1899, 11, 30); // Excel serial 0

export function toSerial(date) {
  return Math.round((date.getTime() - EPOCH) / DAY_MS);
}
export function fromSerial(serial) {
  return new Date(EPOCH + Math.round(serial) * DAY_MS);
}
function dateArg(v) {
  const num = toNumber(v);
  if (isErr(num)) {
    const parsed = Date.parse(toText(v));
    if (Number.isNaN(parsed)) return E.VALUE();
    return toSerial(new Date(parsed));
  }
  return num;
}

/* ---------------------------------------------------------- number format */

function roundTo(value, digits, mode) {
  const d = Math.trunc(digits);
  const f = 10 ** d;
  const x = value * f;
  const r = mode === 'up' ? Math.sign(x) * Math.ceil(Math.abs(x))
    : mode === 'down' ? Math.sign(x) * Math.floor(Math.abs(x))
    : Math.sign(x) * Math.round(Math.abs(x) + Number.EPSILON * Math.abs(x));
  return r / f;
}

/* ---------------------------------------------------------------- exports */

export const FUNCTIONS = {
  /* math & aggregation */
  SUM: guard((args) => {
    const nums = numericValues(args);
    return isErr(nums) ? nums : nums.reduce((a, b) => a + b, 0);
  }),
  PRODUCT: guard((args) => {
    const nums = numericValues(args);
    if (isErr(nums)) return nums;
    return nums.length ? nums.reduce((a, b) => a * b, 1) : 0;
  }),
  AVERAGE: guard((args) => {
    const nums = numericValues(args);
    if (isErr(nums)) return nums;
    return nums.length ? nums.reduce((a, b) => a + b, 0) / nums.length : E.DIV0();
  }),
  MEDIAN: guard((args) => {
    const nums = numericValues(args);
    if (isErr(nums)) return nums;
    if (!nums.length) return E.NUM();
    const s = [...nums].sort((a, b) => a - b);
    const mid = Math.floor(s.length / 2);
    return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
  }),
  MIN: guard((args) => {
    const nums = numericValues(args);
    if (isErr(nums)) return nums;
    return nums.length ? Math.min(...nums) : 0;
  }),
  MAX: guard((args) => {
    const nums = numericValues(args);
    if (isErr(nums)) return nums;
    return nums.length ? Math.max(...nums) : 0;
  }),
  COUNT: (args) => {
    const nums = numericValues(args);
    return isErr(nums) ? 0 : nums.length;
  },
  COUNTA: (args) => flatten(args).filter((v) => v !== null && v !== undefined && v !== '').length,
  COUNTBLANK: (args) => flatten(args).filter((v) => v === null || v === undefined || v === '').length,
  ABS: guard((args) => wrap1(args, Math.abs)),
  SQRT: guard((args) => {
    const x = n(args[0]);
    if (isErr(x)) return x;
    return x < 0 ? E.NUM() : Math.sqrt(x);
  }),
  POWER: guard((args) => bin(args, (a, b) => a ** b)),
  MOD: guard((args) => bin(args, (a, b) => (b === 0 ? E.DIV0() : a - b * Math.floor(a / b)))),
  INT: guard((args) => wrap1(args, Math.floor)),
  TRUNC: guard((args) => {
    const x = n(args[0]);
    const d = args.length > 1 ? n(args[1]) : 0;
    if (isErr(x)) return x;
    if (isErr(d)) return d;
    return roundTo(x, d, 'down');
  }),
  ROUND: guard((args) => {
    const x = n(args[0]);
    const d = args.length > 1 ? n(args[1]) : 0;
    if (isErr(x)) return x;
    if (isErr(d)) return d;
    return roundTo(x, d, 'half');
  }),
  ROUNDUP: guard((args) => roundArg(args, 'up')),
  ROUNDDOWN: guard((args) => roundArg(args, 'down')),
  CEILING: guard((args) => bin(args, (a, b) => (b === 0 ? 0 : Math.ceil(a / b) * b))),
  FLOOR: guard((args) => bin(args, (a, b) => (b === 0 ? E.DIV0() : Math.floor(a / b) * b))),
  SIGN: guard((args) => wrap1(args, Math.sign)),
  EXP: guard((args) => wrap1(args, Math.exp)),
  LN: guard((args) => {
    const x = n(args[0]);
    if (isErr(x)) return x;
    return x <= 0 ? E.NUM() : Math.log(x);
  }),
  LOG10: guard((args) => {
    const x = n(args[0]);
    if (isErr(x)) return x;
    return x <= 0 ? E.NUM() : Math.log10(x);
  }),
  RAND: () => Math.random(),
  PI: () => Math.PI,
  SUMPRODUCT: guard((args) => {
    const arrays = args.map((a) => (isRange(a) ? a.values() : [a]));
    const len = Math.max(...arrays.map((a) => a.length));
    let total = 0;
    for (let i = 0; i < len; i++) {
      let p = 1;
      for (const arr of arrays) {
        const v = toNumber(arr[i] ?? 0);
        if (isErr(v)) return v;
        p *= v;
      }
      total += p;
    }
    return total;
  }),
  STDEV: guard((args) => {
    const nums = numericValues(args);
    if (isErr(nums)) return nums;
    if (nums.length < 2) return E.DIV0();
    const mean = nums.reduce((a, b) => a + b, 0) / nums.length;
    return Math.sqrt(nums.reduce((a, b) => a + (b - mean) ** 2, 0) / (nums.length - 1));
  }),

  /* conditional aggregation */
  SUMIF: (args) => {
    const picked = pairedIf(args[0], args[1], args[2]);
    if (isErr(picked)) return picked;
    const nums = numericValues([picked]);
    return isErr(nums) ? nums : nums.reduce((a, b) => a + b, 0);
  },
  COUNTIF: (args) => {
    if (!isRange(args[0])) return E.VALUE();
    const test = makeCriteria(args[1]);
    return args[0].values().filter(test).length;
  },
  AVERAGEIF: (args) => {
    const picked = pairedIf(args[0], args[1], args[2]);
    if (isErr(picked)) return picked;
    const nums = numericValues([picked]);
    if (isErr(nums)) return nums;
    return nums.length ? nums.reduce((a, b) => a + b, 0) / nums.length : E.DIV0();
  },

  /* logic */
  IF: (args, ctx) => {
    const cond = toBoolean(args[0]);
    if (isErr(cond)) return cond;
    const branch = cond ? args[1] : args[2];
    if (branch === undefined) return cond;
    return isRange(branch) ? branch.values()[0] ?? null : branch;
  },
  IFERROR: (args) => (isErr(args[0]) || (isRange(args[0]) && isErr(args[0].values()[0])) ? args[1] ?? null : args[0]),
  IFNA: (args) => {
    const v = args[0];
    return isErr(v) && v.code === '#N/A' ? args[1] ?? null : v;
  },
  IFS: (args) => {
    for (let i = 0; i + 1 < args.length; i += 2) {
      const c = toBoolean(args[i]);
      if (isErr(c)) return c;
      if (c) return args[i + 1];
    }
    return E.NA();
  },
  AND: guard((args) => {
    const vals = flatten(args).filter((v) => v !== null && v !== '');
    for (const v of vals) {
      const b = toBoolean(v);
      if (isErr(b)) return b;
      if (!b) return false;
    }
    return true;
  }),
  OR: guard((args) => {
    const vals = flatten(args).filter((v) => v !== null && v !== '');
    for (const v of vals) {
      const b = toBoolean(v);
      if (isErr(b)) return b;
      if (b) return true;
    }
    return false;
  }),
  NOT: guard((args) => {
    const b = toBoolean(args[0]);
    return isErr(b) ? b : !b;
  }),
  TRUE: () => true,
  FALSE: () => false,
  ISBLANK: (args) => args[0] === null || args[0] === undefined || args[0] === '',
  ISNUMBER: (args) => typeof (isRange(args[0]) ? args[0].values()[0] : args[0]) === 'number',
  ISTEXT: (args) => typeof (isRange(args[0]) ? args[0].values()[0] : args[0]) === 'string',
  ISERROR: (args) => isErr(args[0]) || (isRange(args[0]) && isErr(args[0].values()[0])),
  NA: () => E.NA(),

  /* text */
  CONCAT: (args) => flatten(args).map(toText).join(''),
  CONCATENATE: (args) => flatten(args).map(toText).join(''),
  TEXTJOIN: (args) => {
    const sep = toText(args[0]);
    const skipEmpty = toBoolean(args[1]) === true;
    const vals = flatten(args.slice(2)).filter((v) => !skipEmpty || (v !== null && v !== ''));
    return vals.map(toText).join(sep);
  },
  LEN: guard((args) => toText(args[0]).length),
  LEFT: guard((args) => sliceText(args, 'left')),
  RIGHT: guard((args) => sliceText(args, 'right')),
  MID: guard((args) => {
    const s = toText(args[0]);
    const start = n(args[1]);
    const len = n(args[2]);
    if (isErr(start)) return start;
    if (isErr(len)) return len;
    if (start < 1 || len < 0) return E.VALUE();
    return s.slice(start - 1, start - 1 + Math.trunc(len));
  }),
  UPPER: guard((args) => toText(args[0]).toUpperCase()),
  LOWER: guard((args) => toText(args[0]).toLowerCase()),
  PROPER: guard((args) => toText(args[0]).replace(/\S+/g, (w) => w[0].toUpperCase() + w.slice(1).toLowerCase())),
  TRIM: guard((args) => toText(args[0]).replace(/\s+/g, ' ').trim()),
  SUBSTITUTE: guard((args) => {
    const s = toText(args[0]);
    const find = toText(args[1]);
    const repl = toText(args[2]);
    if (!find) return s;
    if (args[3] === undefined) return s.split(find).join(repl);
    const nth = Math.trunc(toNumber(args[3]));
    let i = -1;
    let count = 0;
    while ((i = s.indexOf(find, i + 1)) !== -1) {
      if (++count === nth) return s.slice(0, i) + repl + s.slice(i + find.length);
    }
    return s;
  }),
  FIND: guard((args) => {
    const pos = toText(args[1]).indexOf(toText(args[0]), Math.max(0, Math.trunc(toNumber(args[2] ?? 1)) - 1));
    return pos < 0 ? E.VALUE() : pos + 1;
  }),
  VALUE: guard((args) => n(args[0])),
  TEXT: guard((args) => {
    const v = isRange(args[0]) ? args[0].values()[0] : args[0];
    return applyNumFmt(v, toText(args[1]));
  }),

  /* lookup & reference */
  VLOOKUP: (args) => lookupIn(args[0], args[1], args[2], args[3] === undefined ? false : toBoolean(args[3]) === true, false),
  HLOOKUP: (args) => lookupIn(args[0], args[1], args[2], args[3] === undefined ? false : toBoolean(args[3]) === true, true),
  INDEX: (args) => {
    if (!isRange(args[0])) return E.VALUE();
    const grid = args[0].grid();
    const r = Math.trunc(toNumber(args[1] ?? 1));
    const c = args[2] === undefined ? 1 : Math.trunc(toNumber(args[2]));
    if (isErr(r) || isErr(c)) return E.VALUE();
    // A single-row or single-column range accepts one index.
    if (args[2] === undefined && grid.length === 1) return grid[0][r - 1] ?? E.REF();
    if (args[2] === undefined && (grid[0] ?? []).length === 1) return grid[r - 1]?.[0] ?? E.REF();
    const v = grid[r - 1]?.[c - 1];
    return v === undefined ? E.REF() : v;
  },
  MATCH: (args) => {
    if (!isRange(args[1])) return E.VALUE();
    const values = args[1].values();
    const type = args[2] === undefined ? 1 : Math.trunc(toNumber(args[2]));
    if (type === 0) {
      const test = makeCriteria(args[0]);
      const i = values.findIndex(test);
      return i < 0 ? E.NA() : i + 1;
    }
    let best = -1;
    for (let i = 0; i < values.length; i++) {
      const c = compareValues(values[i], args[0]);
      if (type === 1 && c <= 0) best = i;
      if (type === -1 && c >= 0) best = i;
    }
    return best < 0 ? E.NA() : best + 1;
  },
  ROWS: (args) => (isRange(args[0]) ? args[0].rows : 1),
  COLUMNS: (args) => (isRange(args[0]) ? args[0].cols : 1),

  /* date & time */
  TODAY: () => toSerial(new Date()),
  NOW: () => (Date.now() - Date.UTC(1899, 11, 30)) / DAY_MS,
  DATE: guard((args) => {
    const y = Math.trunc(toNumber(args[0]));
    const m = Math.trunc(toNumber(args[1]));
    const d = Math.trunc(toNumber(args[2]));
    if (isErr(y) || isErr(m) || isErr(d)) return E.VALUE();
    return toSerial(new Date(Date.UTC(y, m - 1, d)));
  }),
  YEAR: guard((args) => datePart(args[0], (d) => d.getUTCFullYear())),
  MONTH: guard((args) => datePart(args[0], (d) => d.getUTCMonth() + 1)),
  DAY: guard((args) => datePart(args[0], (d) => d.getUTCDate())),
  WEEKDAY: guard((args) => datePart(args[0], (d) => d.getUTCDay() + 1)),
  DAYS: guard((args) => {
    const a = dateArg(args[0]);
    const b = dateArg(args[1]);
    if (isErr(a)) return a;
    if (isErr(b)) return b;
    return a - b;
  }),
};

/* --------------------------------------------------------------- helpers */

function wrap1(args, fn) {
  const x = n(args[0]);
  return isErr(x) ? x : fn(x);
}
function bin(args, fn) {
  const a = n(args[0]);
  const b = n(args[1]);
  if (isErr(a)) return a;
  if (isErr(b)) return b;
  return fn(a, b);
}
function roundArg(args, mode) {
  const x = n(args[0]);
  const d = args.length > 1 ? n(args[1]) : 0;
  if (isErr(x)) return x;
  if (isErr(d)) return d;
  return roundTo(x, d, mode);
}
function sliceText(args, side) {
  const s = toText(args[0]);
  const count = args.length > 1 ? n(args[1]) : 1;
  if (isErr(count)) return count;
  const c = Math.trunc(count);
  if (c < 0) return E.VALUE();
  return side === 'left' ? s.slice(0, c) : c === 0 ? '' : s.slice(-c);
}
function datePart(v, pick) {
  const serial = dateArg(v);
  if (isErr(serial)) return serial;
  return pick(fromSerial(serial));
}

/**
 * Minimal number-format renderer covering the patterns the UI offers:
 * `#,##0`, `#,##0.00`, `0%`, `0.0%`, `₩#,##0`, `$#,##0.00`, `yyyy-mm-dd`, `@`.
 */
export function applyNumFmt(value, fmt) {
  if (value === null || value === undefined || value === '') return '';
  if (isErr(value)) return value.code;
  if (!fmt || fmt === 'General' || fmt === '@') {
    return typeof value === 'number' ? trimNum(value) : toText(value);
  }
  if (/[ymd]/i.test(fmt) && !/[#0]/.test(fmt)) {
    const serial = dateArg(value);
    if (isErr(serial)) return toText(value);
    const d = fromSerial(serial);
    const pad2 = (x) => String(x).padStart(2, '0');
    return fmt
      .replace(/yyyy/gi, String(d.getUTCFullYear()))
      .replace(/yy/gi, String(d.getUTCFullYear()).slice(2))
      .replace(/mm/g, pad2(d.getUTCMonth() + 1))
      .replace(/dd/gi, pad2(d.getUTCDate()));
  }

  const num = toNumber(value);
  if (isErr(num)) return toText(value);

  const isPercent = fmt.includes('%');
  const scaled = isPercent ? num * 100 : num;
  const decimals = (fmt.match(/\.(0+)/) ?? [, ''])[1].length;
  const useGrouping = fmt.includes('#,##');
  const prefix = (fmt.match(/^[^#0.,%]*/) ?? [''])[0];
  const suffix = (fmt.match(/[^#0.,%]*$/) ?? [''])[0];

  const body = Math.abs(scaled).toLocaleString('en-US', {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
    useGrouping,
  });
  const sign = scaled < 0 ? '-' : '';
  return `${sign}${prefix}${body}${isPercent ? '%' : ''}${suffix}`;
}

function trimNum(x) {
  if (Number.isInteger(x)) return String(x);
  return String(Number(x.toPrecision(12)));
}

export { E as ERRORS_FACTORY };
