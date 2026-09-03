import {
  toRef, parseRef, parseRange, indexToCol, adjustRefs, shiftFormula,
  recalcSheet, parseCellInput, displayValue, editValue, LIMITS,
} from '../core/index.js';

/**
 * Recalculate after any mutation so displayed values never lag the formulas.
 *
 * `others` are the workbook's remaining sheets. Without them a cross-sheet
 * formula cannot be resolved and the core leaves the value already in the cell,
 * so calling this without the workbook is safe but incomplete — the editor runs
 * a final pass with the workbook after each edit.
 */
export function withRecalc(sheet, others = []) {
  const { cells } = recalcSheet(
    { cells: sheet.cells, names: sheet.names, name: sheet.name, merges: sheet.merges },
    others
  );
  // A dynamic array can spill past the grid's current edge; grow the grid so
  // every spilled value is on screen rather than silently below the fold.
  let dims = sheet.dims;
  for (const [ref, cell] of Object.entries(cells)) {
    if (!cell?.spillFrom) continue;
    const at = parseRef(ref);
    if (at) dims = growDims(dims, at.row, at.col);
  }
  return { ...sheet, cells, dims };
}

export function setCellInput(sheet, row, col, raw) {
  const ref = toRef(col, row);
  const parsed = parseCellInput(raw);
  const cells = { ...sheet.cells };
  const existing = cells[ref];

  if (!parsed) {
    // Clearing the content keeps the styling, as Excel's Delete key does.
    if (existing?.style) cells[ref] = { style: existing.style };
    else delete cells[ref];
  } else {
    cells[ref] = {
      ...(existing?.style ? { style: existing.style } : {}),
      ...(existing?.fmt && !parsed.fmt ? { fmt: existing.fmt } : {}),
      ...parsed,
    };
  }
  return withRecalc({ ...sheet, cells, dims: growDims(sheet.dims, row, col) });
}

export function patchCells(sheet, refs, patch) {
  const cells = { ...sheet.cells };
  for (const ref of refs) {
    const existing = cells[ref] ?? {};
    const next = { ...existing };
    if (patch.style) {
      const style = { ...(existing.style ?? {}), ...patch.style };
      for (const [k, v] of Object.entries(patch.style)) {
        // Passing null removes a style property rather than storing null.
        if (v === null || v === false) delete style[k];
      }
      if (Object.keys(style).length) next.style = style;
      else delete next.style;
    }
    if ('fmt' in patch) {
      if (patch.fmt) next.fmt = patch.fmt;
      else delete next.fmt;
    }
    const isEmpty = !next.f && (next.v === null || next.v === undefined || next.v === '') && !next.style && !next.fmt;
    if (isEmpty) delete cells[ref];
    else cells[ref] = next;
  }
  return withRecalc({ ...sheet, cells });
}

/**
 * 서식 복사 — put one cell's whole look onto others.
 *
 * This replaces rather than merges, which is the difference between the format
 * painter and the individual buttons: painting a plain cell's format onto a bold
 * one has to un-bold it, or the brush can only ever add.
 */
export function paintFormat(sheet, refs, { style, fmt }) {
  const cells = { ...sheet.cells };
  for (const ref of refs) {
    const existing = cells[ref];
    if (!existing) {
      // Nothing to paint onto: an empty cell stays out of the file unless the
      // brush actually carries something.
      if (!style && !fmt) continue;
      cells[ref] = {
        v: null,
        ...(style ? { style: { ...style } } : {}),
        ...(fmt ? { fmt } : {}),
      };
      continue;
    }
    const next = { ...existing };
    delete next.style;
    delete next.fmt;
    if (style && Object.keys(style).length) next.style = { ...style };
    if (fmt) next.fmt = fmt;
    const isEmpty = !next.f && (next.v === null || next.v === undefined || next.v === '') && !next.style && !next.fmt;
    if (isEmpty) delete cells[ref];
    else cells[ref] = next;
  }
  return withRecalc({ ...sheet, cells });
}

export function clearRange(sheet, refs, { keepStyle = false } = {}) {
  const cells = { ...sheet.cells };
  for (const ref of refs) {
    const existing = cells[ref];
    if (!existing) continue;
    if (keepStyle && existing.style) cells[ref] = { style: existing.style };
    else delete cells[ref];
  }
  return withRecalc({ ...sheet, cells });
}

/* ------------------------------------------------------- rows and columns */

/**
 * Insert or delete whole rows/columns, moving the cells and rewriting the
 * formulas that referenced them.
 */
export function structuralEdit(sheet, axis, at, delta) {
  const cells = {};
  const shift = axis === 'row' ? { dr: delta, dc: 0 } : { dr: 0, dc: delta };

  for (const [ref, cell] of Object.entries(sheet.cells)) {
    const p = parseRef(ref);
    if (!p) continue;
    const index = axis === 'row' ? p.row : p.col;

    if (delta < 0 && index >= at && index < at - delta) continue; // removed
    const moved = index >= at;
    const next = {
      row: p.row + (moved ? shift.dr : 0),
      col: p.col + (moved ? shift.dc : 0),
    };
    if (next.row < 0 || next.col < 0) continue;

    // Every formula is rewritten, moved or not: a formula that stayed put can
    // still reference a cell that shifted.
    const rewritten = cell.f ? { ...cell, f: adjustRefs(cell.f, axis, at, delta) } : cell;
    cells[toRef(next.col, next.row)] = rewritten;
  }

  const sizeMap = axis === 'row' ? 'rowHeights' : 'colWidths';
  const dims = {
    rows: axis === 'row' ? Math.max(1, sheet.dims.rows + delta) : sheet.dims.rows,
    cols: axis === 'col' ? Math.max(1, sheet.dims.cols + delta) : sheet.dims.cols,
  };

  return withRecalc({
    ...sheet,
    cells,
    dims,
    [sizeMap]: shiftSizeMap(sheet[sizeMap], axis, at, delta),
  });
}

function shiftSizeMap(map, axis, at, delta) {
  const out = {};
  for (const [key, value] of Object.entries(map ?? {})) {
    const index = axis === 'row' ? Number(key) - 1 : colIndexOf(key);
    if (!Number.isFinite(index)) continue;
    if (delta < 0 && index >= at && index < at - delta) continue;
    const next = index >= at ? index + delta : index;
    if (next < 0) continue;
    out[axis === 'row' ? String(next + 1) : indexToCol(next)] = value;
  }
  return out;
}

function colIndexOf(letters) {
  let n = 0;
  for (const ch of String(letters).toUpperCase()) {
    const c = ch.charCodeAt(0) - 65;
    if (c < 0 || c > 25) return NaN;
    n = n * 26 + c + 1;
  }
  return n - 1;
}

function growDims(dims, row, col) {
  return {
    rows: Math.max(dims?.rows ?? 200, row + 1),
    cols: Math.max(dims?.cols ?? 26, col + 1),
  };
}

/* ----------------------------------------------------------- clipboard */

/** Tab-separated text of a range, so it pastes into Excel and Sheets as-is. */
export function rangeToTsv(sheet, range) {
  const rows = [];
  for (let r = range.r1; r <= range.r2; r++) {
    const row = [];
    for (let c = range.c1; c <= range.c2; c++) {
      row.push(displayValue(sheet.cells[toRef(c, r)]).replace(/\t/g, ' '));
    }
    rows.push(row.join('\t'));
  }
  return rows.join('\n');
}

/** Formulas rather than values — Excel's Ctrl+` view, useful for AI hand-off. */
export function rangeToFormulaTsv(sheet, range) {
  const rows = [];
  for (let r = range.r1; r <= range.r2; r++) {
    const row = [];
    for (let c = range.c1; c <= range.c2; c++) {
      row.push(editValue(sheet.cells[toRef(c, r)]).replace(/\t/g, ' '));
    }
    rows.push(row.join('\t'));
  }
  return rows.join('\n');
}

export function pasteTsv(sheet, text, startRow, startCol) {
  const lines = String(text ?? '').replace(/\r\n?/g, '\n').replace(/\n$/, '').split('\n');
  let next = sheet;
  lines.forEach((line, r) => {
    line.split('\t').forEach((value, c) => {
      next = setCellInput(next, startRow + r, startCol + c, value);
    });
  });
  return next;
}

/* ---------------------------------------------------------------- ranges */

export function normalizeRange(sel) {
  return {
    r1: Math.min(sel.row, sel.row2 ?? sel.row),
    r2: Math.max(sel.row, sel.row2 ?? sel.row),
    c1: Math.min(sel.col, sel.col2 ?? sel.col),
    c2: Math.max(sel.col, sel.col2 ?? sel.col),
  };
}

export function rangeRefs(range) {
  const refs = [];
  for (let r = range.r1; r <= range.r2; r++) {
    for (let c = range.c1; c <= range.c2; c++) refs.push(toRef(c, r));
  }
  return refs;
}

export function rangeLabel(range) {
  if (range.r1 === range.r2 && range.c1 === range.c2) return toRef(range.c1, range.r1);
  return `${toRef(range.c1, range.r1)}:${toRef(range.c2, range.r2)}`;
}

/** Live aggregate of the selection, like Excel's status bar. */
export function selectionStats(sheet, range) {
  let count = 0;
  let numeric = 0;
  let sum = 0;
  for (const ref of rangeRefs(range)) {
    const cell = sheet.cells[ref];
    if (!cell || cell.v === null || cell.v === undefined || cell.v === '') continue;
    count++;
    if (typeof cell.v === 'number' && cell.t !== 'e') {
      numeric++;
      sum += cell.v;
    }
  }
  return { count, numeric, sum, avg: numeric ? sum / numeric : null };
}

/* -------------------------------------------------------------- navigation */

const isFilled = (sheet, row, col) => {
  const cell = sheet.cells[toRef(col, row)];
  return !!cell && cell.v !== null && cell.v !== undefined && cell.v !== '';
};

/**
 * Where Ctrl+Arrow lands, by Excel's rule.
 *
 * Inside a run of filled cells it goes to the last one; standing at the edge of
 * a run it skips the blanks and lands on the next filled cell; with nothing
 * ahead it goes to the last row or column of the sheet. This is how anyone gets
 * to the bottom of a ten-thousand-row table, and holding the arrow key instead
 * is not a substitute.
 */
export function edgeOf(sheet, row, col, direction) {
  const step = { up: [-1, 0], down: [1, 0], left: [0, -1], right: [0, 1] }[direction];
  if (!step) return { row, col };
  const [dr, dc] = step;
  const lastRow = Math.max(0, (sheet.dims?.rows ?? 200) - 1);
  const lastCol = Math.max(0, (sheet.dims?.cols ?? 26) - 1);
  const inside = (r, c) => r >= 0 && c >= 0 && r <= lastRow && c <= lastCol;

  let r = row;
  let c = col;
  const nextFilled = isFilled(sheet, r + dr, c + dc);
  if (isFilled(sheet, r, c) && nextFilled) {
    // Run to the far end of the block we are standing in.
    while (inside(r + dr, c + dc) && isFilled(sheet, r + dr, c + dc)) {
      r += dr;
      c += dc;
    }
    return { row: r, col: c };
  }
  // Skip the gap and land on the next thing there is.
  while (inside(r + dr, c + dc)) {
    r += dr;
    c += dc;
    if (isFilled(sheet, r, c)) return { row: r, col: c };
  }
  return { row: r, col: c };
}

/**
 * The block of filled cells the cursor is standing in — Excel's "current
 * region", which is what Ctrl+A selects first and what Ctrl+Shift+L would
 * filter. An empty cell gives back just itself.
 */
export function currentRegion(sheet, row, col) {
  if (!isFilled(sheet, row, col)) return { r1: row, r2: row, c1: col, c2: col };
  const region = { r1: row, r2: row, c1: col, c2: col };
  // Grow one edge at a time until no edge has anything against it.
  for (let guard = 0; guard < 4096; guard++) {
    let grew = false;
    const anyFilled = (rs, re, cs, ce) => {
      for (let r = rs; r <= re; r++) {
        for (let c = cs; c <= ce; c++) if (isFilled(sheet, r, c)) return true;
      }
      return false;
    };
    if (region.r1 > 0 && anyFilled(region.r1 - 1, region.r1 - 1, region.c1, region.c2)) {
      region.r1--;
      grew = true;
    }
    if (anyFilled(region.r2 + 1, region.r2 + 1, region.c1, region.c2)) {
      region.r2++;
      grew = true;
    }
    if (region.c1 > 0 && anyFilled(region.r1, region.r2, region.c1 - 1, region.c1 - 1)) {
      region.c1--;
      grew = true;
    }
    if (anyFilled(region.r1, region.r2, region.c2 + 1, region.c2 + 1)) {
      region.c2++;
      grew = true;
    }
    if (!grew) break;
  }
  return region;
}

/**
 * Ctrl+D / Ctrl+R — copy the selection's first row or column over the rest.
 *
 * Excel treats it as a fill from that first line, so a formula picks up shifted
 * references exactly as a drag of the fill handle would.
 */
export function fillWithin(sheet, range, direction) {
  if (direction === 'down') {
    if (range.r2 <= range.r1) return sheet;
    return fillRange(sheet, { ...range, r2: range.r1 }, { ...range, dir: 'down' });
  }
  if (range.c2 <= range.c1) return sheet;
  return fillRange(sheet, { ...range, c2: range.c1 }, { ...range, dir: 'right' });
}

/**
 * Toggle the `$` signs in a formula's references, cycling A1 → $A$1 → A$1 →
 * $A1 → A1 — Excel's F4, pressed constantly while writing a lookup.
 *
 * `caret` selects the reference under the cursor; without one every reference
 * in the formula is cycled, which is what F4 does to a selected reference.
 */
export function cycleRefLocks(formula, caret) {
  const text = String(formula ?? '');
  if (!text.startsWith('=')) return null;
  const pattern = /(\$?)([A-Za-z]{1,3})(\$?)(\d{1,7})/g;
  let hit = null;
  for (let m = pattern.exec(text); m; m = pattern.exec(text)) {
    const start = m.index;
    const end = start + m[0].length;
    if (caret === undefined || caret === null || (caret >= start && caret <= end)) {
      hit = { start, end, colLock: m[1], col: m[2], rowLock: m[3], row: m[4] };
      if (caret !== undefined && caret !== null) break;
    }
  }
  if (!hit) return null;
  // A1 → $A$1 → A$1 → $A1 → A1
  const state = `${hit.colLock ? 'c' : ''}${hit.rowLock ? 'r' : ''}`;
  const next = { '': '$c$r', cr: 'r', r: 'c', c: '' }[state] ?? '';
  const locked = { '$c$r': ['$', '$'], r: ['', '$'], c: ['$', ''], '': ['', ''] }[next];
  const replaced = `${locked[0]}${hit.col}${locked[1]}${hit.row}`;
  return {
    value: text.slice(0, hit.start) + replaced + text.slice(hit.end),
    caret: hit.start + replaced.length,
  };
}

/* ------------------------------------------------------------- fill handle */

/**
 * Where a fill drag lands.
 *
 * Excel extends the source rectangle along whichever axis the pointer moved
 * furthest, and never shrinks it — so the result is always the source plus one
 * contiguous strip.
 */
export function fillTarget(source, row, col) {
  const belowBy = row - source.r2;
  const aboveBy = source.r1 - row;
  const rightBy = col - source.c2;
  const leftBy = source.c1 - col;

  const vertical = Math.max(belowBy, aboveBy);
  const horizontal = Math.max(rightBy, leftBy);
  if (vertical <= 0 && horizontal <= 0) return null;

  if (vertical >= horizontal) {
    return belowBy >= aboveBy
      ? { ...source, r2: row, dir: 'down' }
      : { ...source, r1: row, dir: 'up' };
  }
  return rightBy >= leftBy
    ? { ...source, c2: col, dir: 'right' }
    : { ...source, c1: col, dir: 'left' };
}

/**
 * Trailing-integer text series: "항목 1" -> stem "항목 ", number 1.
 *
 * The digits are read unsigned on purpose. A genuinely negative number would have
 * been stored as a number, not text, so a leading `-` here is punctuation —
 * treating "ID-008" as minus eight made the series count *down*.
 */
function splitTrailingNumber(text) {
  const m = String(text).match(/^(.*?)(\d+)$/);
  return m ? { stem: m[1], n: Number(m[2]), width: m[2].length } : null;
}

/**
 * Project one line (a column when filling vertically, a row when filling
 * horizontally) from its source cells into `count` new positions.
 *
 * Rules follow Excel closely enough to be unsurprising:
 * - formulas shift their relative references
 * - two or more numbers extrapolate linearly by the average step
 * - one number copies (Excel does not increment a lone number)
 * - text ending in digits increments
 * - anything else repeats, cycling through the source
 */
function projectLine(sourceCells, count, reverse, stepVector) {
  const cells = reverse ? [...sourceCells].reverse() : sourceCells;
  const len = cells.length;
  // Filling up or left counts backwards for series that have no step to infer.
  const sign = reverse ? -1 : 1;

  const numbers = cells.map((c) => (c && typeof c.v === 'number' && !c.f ? c.v : null));
  const allNumeric = len > 0 && numbers.every((n) => n !== null);

  let step = 0;
  if (allNumeric && len >= 2) {
    let total = 0;
    for (let i = 1; i < len; i++) total += numbers[i] - numbers[i - 1];
    step = total / (len - 1);
  }
  const last = allNumeric ? numbers[len - 1] : null;

  const out = [];
  for (let i = 0; i < count; i++) {
    const src = cells[i % len];
    if (!src) {
      out.push(null);
      continue;
    }
    const next = { ...src };
    delete next.note;

    if (src.f) {
      const cycles = i + 1;
      const dc = stepVector.dc * cycles;
      const dr = stepVector.dr * cycles;
      next.f = shiftFormula(src.f, dc, dr);
      next.v = null;
    } else if (allNumeric && len >= 2) {
      next.v = round12(last + step * (i + 1));
      next.t = src.t === 'd' ? 'd' : 'n';
    } else if (typeof src.v === 'string') {
      const parts = splitTrailingNumber(src.v);
      if (parts) {
        const bump = (Math.floor(i / len) + 1) * sign;
        const value = parts.n + bump;
        const digits = String(Math.abs(value)).padStart(parts.width, '0');
        next.v = `${parts.stem}${value < 0 ? '-' : ''}${digits}`;
      }
    } else if (allNumeric && len === 1 && src.t === 'd') {
      // Dates are the one single-cell case Excel does increment.
      next.v = src.v + (i + 1) * sign;
    }
    out.push(next);
  }
  // Index i is the i-th step away from the source edge, and the caller places it
  // that way, so the order produced here is already the order needed.
  return out;
}

function round12(n) {
  return Number(Number(n).toPrecision(12));
}

/**
 * Apply a fill from `source` to the strip added by `target`.
 * Returns the sheet unchanged when the target adds nothing.
 */
export function fillRange(sheet, source, target) {
  const dir = target.dir ?? inferDir(source, target);
  if (!dir) return sheet;
  const cells = { ...sheet.cells };
  const vertical = dir === 'down' || dir === 'up';

  const lines = vertical
    ? rangeSeq(source.c1, source.c2)
    : rangeSeq(source.r1, source.r2);

  const count = vertical
    ? dir === 'down' ? target.r2 - source.r2 : source.r1 - target.r1
    : dir === 'right' ? target.c2 - source.c2 : source.c1 - target.c1;
  if (count <= 0) return sheet;

  const stepVector = vertical
    ? { dc: 0, dr: dir === 'down' ? 1 : -1 }
    : { dc: dir === 'right' ? 1 : -1, dr: 0 };

  for (const line of lines) {
    const sourceCells = vertical
      ? rangeSeq(source.r1, source.r2).map((r) => cells[toRef(line, r)] ?? null)
      : rangeSeq(source.c1, source.c2).map((c) => cells[toRef(c, line)] ?? null);

    const reverse = dir === 'up' || dir === 'left';
    const produced = projectLine(sourceCells, count, reverse, stepVector);

    produced.forEach((cell, i) => {
      const offset = i + 1;
      const [r, c] = vertical
        ? [dir === 'down' ? source.r2 + offset : source.r1 - offset, line]
        : [line, dir === 'right' ? source.c2 + offset : source.c1 - offset];
      if (r < 0 || c < 0) return;
      const ref = toRef(c, r);
      if (cell) cells[ref] = cell;
      else delete cells[ref];
    });
  }

  const grown = {
    rows: Math.max(sheet.dims.rows, target.r2 + 1),
    cols: Math.max(sheet.dims.cols, target.c2 + 1),
  };
  return withRecalc({ ...sheet, cells, dims: grown });
}

function inferDir(source, target) {
  if (target.r2 > source.r2) return 'down';
  if (target.r1 < source.r1) return 'up';
  if (target.c2 > source.c2) return 'right';
  if (target.c1 < source.c1) return 'left';
  return null;
}

function rangeSeq(from, to) {
  const out = [];
  for (let i = from; i <= to; i++) out.push(i);
  return out;
}

/* ------------------------------------------------------------------ merges */

/** The merge rectangle covering a cell, or null. Merges are stored as "A1:B2". */
export function mergeCovering(sheet, row, col) {
  for (const spec of sheet.merges ?? []) {
    const r = parseRange(spec);
    if (!r) continue;
    if (row >= r.start.row && row <= r.end.row && col >= r.start.col && col <= r.end.col) {
      return { spec, r1: r.start.row, c1: r.start.col, r2: r.end.row, c2: r.end.col };
    }
  }
  return null;
}

/**
 * Merge the selection into one cell.
 * Content outside the top-left corner is dropped, which is what Excel warns
 * about and then does.
 */
export function mergeSelection(sheet, range) {
  if (range.r1 === range.r2 && range.c1 === range.c2) return sheet;
  const spec = `${toRef(range.c1, range.r1)}:${toRef(range.c2, range.r2)}`;
  const cells = { ...sheet.cells };

  for (const ref of rangeRefs(range)) {
    if (ref === toRef(range.c1, range.r1)) continue;
    const existing = cells[ref];
    if (existing?.style) cells[ref] = { style: existing.style };
    else delete cells[ref];
  }

  // Drop any merge that overlaps the new one, so merges never intersect.
  const merges = (sheet.merges ?? []).filter((other) => !overlaps(other, range));
  return withRecalc({ ...sheet, cells, merges: [...merges, spec] });
}

export function unmergeSelection(sheet, range) {
  const merges = (sheet.merges ?? []).filter((spec) => !overlaps(spec, range));
  if (merges.length === (sheet.merges ?? []).length) return sheet;
  return { ...sheet, merges };
}

function overlaps(spec, range) {
  const r = parseRange(spec);
  if (!r) return false;
  return !(
    r.end.row < range.r1 || r.start.row > range.r2 ||
    r.end.col < range.c1 || r.start.col > range.c2
  );
}

/* ----------------------------------------------------------------- borders */

export const BORDER_PRESETS = {
  all: { t: true, r: true, b: true, l: true },
  outline: 'outline',
  bottom: { b: true },
  top: { t: true },
  none: null,
};

/**
 * Apply a border preset to the selection.
 * `outline` draws only around the perimeter, which needs per-cell edges.
 */
export function applyBorders(sheet, range, preset) {
  const cells = { ...sheet.cells };

  for (let r = range.r1; r <= range.r2; r++) {
    for (let c = range.c1; c <= range.c2; c++) {
      const ref = toRef(c, r);
      const existing = cells[ref] ?? {};
      const style = { ...(existing.style ?? {}) };

      if (preset === null) {
        delete style.border;
      } else if (preset === 'outline') {
        const edges = {};
        if (r === range.r1) edges.t = true;
        if (r === range.r2) edges.b = true;
        if (c === range.c1) edges.l = true;
        if (c === range.c2) edges.r = true;
        if (Object.keys(edges).length) style.border = edges;
        else delete style.border;
      } else {
        style.border = { ...preset };
      }

      const next = { ...existing };
      if (Object.keys(style).length) next.style = style;
      else delete next.style;

      const isEmpty =
        !next.f && (next.v === null || next.v === undefined || next.v === '') && !next.style && !next.fmt;
      if (isEmpty) delete cells[ref];
      else cells[ref] = next;
    }
  }
  return { ...sheet, cells };
}

/* ------------------------------------------------------- size adjustments */

export function setColWidth(sheet, col, width) {
  const key = indexToCol(col);
  const next = { ...(sheet.colWidths ?? {}) };
  if (width && width !== LIMITS.colWidth) next[key] = Math.max(24, Math.round(width));
  else delete next[key];
  return { ...sheet, colWidths: next };
}

export function setRowHeight(sheet, row, height) {
  const key = String(row + 1);
  const next = { ...(sheet.rowHeights ?? {}) };
  if (height && height !== LIMITS.rowHeight) next[key] = Math.max(16, Math.round(height));
  else delete next[key];
  return { ...sheet, rowHeights: next };
}

/** Width that fits the widest rendered value in a column, for double-click. */
export function autoFitColumn(sheet, col, maxRow) {
  let widest = 0;
  for (let r = 0; r <= maxRow; r++) {
    const text = displayValue(sheet.cells[toRef(col, r)]);
    if (!text) continue;
    // Rough advance width: CJK glyphs are about twice as wide as latin.
    let units = 0;
    for (const ch of text) units += /[　-鿿가-힯＀-￯]/.test(ch) ? 2 : 1;
    widest = Math.max(widest, units);
  }
  return Math.min(420, Math.max(56, Math.round(widest * 7.4) + 16));
}

/* ------------------------------------------------------- go to / find */

/** Resolve what the user typed in the name box: a ref, a range, or a name. */
/**
 * The height a row needs, given the wrapped cells in it.
 *
 * Only wrapped cells can be taller than one line, so an ordinary row auto-fits
 * back to the default — which is exactly what double-clicking the boundary is
 * for after a wrap has been turned off.
 */
export function autoFitRow(sheet, row, maxCol) {
  let lines = 1;
  for (let c = 0; c <= maxCol; c++) {
    const cell = sheet.cells[toRef(c, row)];
    if (!cell?.style?.wrap) continue;
    const width = sheet.colWidths?.[indexToCol(c)] ?? LIMITS.colWidth;
    const text = String(displayValue(cell) ?? '');
    // ~7px a character at the sheet's own size, which is what the measurement
    // in `autoFitColumn` assumes too.
    const perLine = Math.max(1, Math.floor((width - 10) / 7));
    lines = Math.max(lines, Math.ceil(text.length / perLine));
  }
  return lines <= 1 ? 0 : Math.min(400, lines * (LIMITS.rowHeight - 4) + 4);
}

export function resolveTarget(sheet, input) {
  const text = String(input ?? '').trim();
  if (!text) return null;

  const named = sheet.names?.[text] ?? sheet.names?.[text.toUpperCase()];
  const candidate = named ?? text;

  const range = parseRange(candidate);
  if (range) {
    return {
      row: range.start.row, col: range.start.col,
      row2: range.end.row, col2: range.end.col,
    };
  }
  const ref = parseRef(candidate);
  if (ref) return { row: ref.row, col: ref.col, row2: ref.row, col2: ref.col };
  return null;
}

/**
 * Find cells whose displayed text (or formula) contains `query`.
 * Returns addresses in reading order so Find Next can cycle through them.
 */
export function findCells(sheet, query, { matchCase = false, inFormulas = false } = {}) {
  const needle = matchCase ? String(query) : String(query).toLowerCase();
  if (!needle) return [];
  const hits = [];
  for (const [ref, cell] of Object.entries(sheet.cells)) {
    const haystackRaw = inFormulas && cell.f ? cell.f : displayValue(cell);
    const haystack = matchCase ? haystackRaw : haystackRaw.toLowerCase();
    if (haystack.includes(needle)) {
      const p = parseRef(ref);
      if (p) hits.push({ ref, row: p.row, col: p.col });
    }
  }
  hits.sort((a, b) => a.row - b.row || a.col - b.col);
  return hits;
}

/** Replace text in the given cells, keeping formulas as formulas. */
export function replaceInCells(sheet, hits, query, replacement, { matchCase = false } = {}) {
  let next = sheet;
  const pattern = new RegExp(escapeRegExp(query), matchCase ? 'g' : 'gi');
  for (const hit of hits) {
    const cell = next.cells[hit.ref];
    if (!cell) continue;
    const source = cell.f ?? displayValue(cell);
    const replaced = source.replace(pattern, replacement);
    if (replaced === source) continue;
    next = setCellInput(next, hit.row, hit.col, replaced);
  }
  return next;
}

function escapeRegExp(text) {
  return String(text).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/* ------------------------------------------------------------------- sort */

/**
 * Does this range's first row look like a header?
 *
 * Excel's own guess: text on top of numbers is a header, and so is a bold first
 * row over a plain one. Getting this wrong is what makes a sort look like it
 * destroyed the table — the column titles end up filed alphabetically among the
 * data.
 */
export function looksLikeHeader(sheet, range) {
  if (range.r2 <= range.r1) return false;
  let headerText = 0;
  let bodyNumeric = 0;
  let columns = 0;
  for (let c = range.c1; c <= range.c2; c++) {
    const head = sheet.cells[toRef(c, range.r1)];
    if (!head || head.v === null || head.v === undefined || head.v === '') continue;
    columns++;
    if (typeof head.v === 'string') headerText++;
    for (let r = range.r1 + 1; r <= range.r2; r++) {
      const cell = sheet.cells[toRef(c, r)];
      if (cell && typeof cell.v === 'number') {
        bodyNumeric++;
        break;
      }
    }
  }
  if (!columns) return false;
  if (headerText === columns && bodyNumeric > 0) return true;
  // A bold first row over a non-bold body is a header even when both are text.
  const headBold = Array.from({ length: range.c2 - range.c1 + 1 }, (_, i) =>
    sheet.cells[toRef(range.c1 + i, range.r1)]?.style?.bold
  ).some(Boolean);
  const bodyBold = sheet.cells[toRef(range.c1, range.r1 + 1)]?.style?.bold;
  return headBold && !bodyBold;
}

/**
 * Sort the selected block, carrying whole rows along.
 *
 * Sorts by the column the cursor is in, not always the first one — that is what
 * Excel does, and it is why clicking a column then pressing 오름차순 works. A
 * header row is left where it is.
 */
export function sortRange(sheet, range, ascending, { by = range.c1, hasHeader = false } = {}) {
  const first = range.r1 + (hasHeader ? 1 : 0);
  if (range.r2 <= first) return sheet;
  const keyColumn = Math.min(Math.max(by, range.c1), range.c2) - range.c1;

  const rows = [];
  for (let r = first; r <= range.r2; r++) {
    const cells = [];
    for (let c = range.c1; c <= range.c2; c++) cells.push(sheet.cells[toRef(c, r)] ?? null);
    rows.push(cells);
  }

  const key = (cells) => {
    const v = cells[keyColumn]?.v;
    return v === null || v === undefined ? '' : v;
  };
  rows.sort((a, b) => {
    const av = key(a);
    const bv = key(b);
    // Blanks sink to the bottom in both directions, as they do in Excel.
    if (av === '' && bv !== '') return 1;
    if (bv === '' && av !== '') return -1;
    const numeric = typeof av === 'number' && typeof bv === 'number';
    const cmp = numeric ? av - bv : String(av).localeCompare(String(bv), 'ko');
    return ascending ? cmp : -cmp;
  });

  const cells = { ...sheet.cells };
  rows.forEach((rowCells, i) => {
    rowCells.forEach((cell, j) => {
      const ref = toRef(range.c1 + j, first + i);
      if (cell) cells[ref] = cell;
      else delete cells[ref];
    });
  });
  return withRecalc({ ...sheet, cells });
}

/* --------------------------------------------------------- number formats */

/**
 * One step of Excel's 자릿수 늘림 / 줄임.
 *
 * The pattern keeps whatever prefix and suffix it had — a currency or percent
 * format gains decimals rather than being replaced by a plain number.
 */
export function stepDecimals(fmt, direction) {
  const pattern = String(fmt ?? '') || '#,##0';
  const m = pattern.match(/^(.*?)([#0](?:[,#0]*[#0])?)(?:\.(0+))?(.*)$/);
  if (!m) return pattern;
  const [, head, integer, decimals = '', tail] = m;
  const next = Math.max(0, Math.min(9, decimals.length + direction));
  return `${head}${integer}${next > 0 ? `.${'0'.repeat(next)}` : ''}${tail}`;
}
