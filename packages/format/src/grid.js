import { parseFrontmatter, serializeFrontmatter } from './frontmatter.js';
import { newSheetId, newBlockId } from './ids.js';
import { normalizeChartSpec, resolveChartSpec, chartToMarkdownTable, describeChart } from './chart.js';
import {
  indexToCol, parseRef, toRef,
  recalcSheet, displayValue, parseCellInput,
} from '@ai-studio/formula';

export const DEFAULT_DIMS = { rows: 200, cols: 26 };
export const DEFAULT_COL_WIDTH = 104;
export const DEFAULT_ROW_HEIGHT = 28;

/** How many rows the markdown projection will render before truncating. */
const MD_ROW_LIMIT = 500;

/**
 * Merge a sheet's cells JSON with its markdown projection.
 *
 * The JSON is authoritative — it holds formulas, formats and styles. The markdown
 * is a generated projection. The one exception: when cells JSON is absent (a
 * hand-written markdown table dropped into the project), we import the table so
 * the sheet still opens with data.
 */
export function readSheet({ md = '', cells: cellsJson = null } = {}) {
  const { meta } = parseFrontmatter(md);

  if (cellsJson?.cells && Object.keys(cellsJson.cells).length > 0) {
    return normalizeSheet({
      ...cellsJson,
      id: cellsJson.id || meta.id,
      name: cellsJson.name ?? meta.name,
    });
  }

  const imported = importMarkdownTable(md);
  return normalizeSheet({
    id: cellsJson?.id || meta.id || newSheetId(),
    name: cellsJson?.name ?? meta.name ?? '시트1',
    dims: cellsJson?.dims ?? DEFAULT_DIMS,
    frozen: cellsJson?.frozen,
    colWidths: cellsJson?.colWidths,
    rowHeights: cellsJson?.rowHeights,
    cells: imported,
    names: cellsJson?.names,
  });
}

/** Split a sheet into the cells JSON (source) + markdown projection pair. */
export function writeSheet(sheet) {
  const s = normalizeSheet(sheet);
  const { cells } = recalcSheet({ cells: s.cells, names: s.names });
  const withValues = { ...s, cells };
  return {
    md: renderSheetMarkdown(withValues),
    cells: {
      id: s.id,
      name: s.name,
      dims: s.dims,
      frozen: s.frozen,
      colWidths: s.colWidths,
      rowHeights: s.rowHeights,
      merges: s.merges,
      names: s.names,
      ...(s.charts.length ? { charts: s.charts } : {}),
      cells: stripEmpty(cells),
    },
  };
}

export function normalizeSheet(sheet) {
  const cells = {};
  for (const [ref, cell] of Object.entries(sheet?.cells ?? {})) {
    const parsed = parseRef(ref);
    if (!parsed || !cell) continue;
    const key = toRef(parsed.col, parsed.row);
    const next = {};
    if (typeof cell.f === 'string' && cell.f.trim().startsWith('=')) next.f = cell.f.trim();
    if (cell.v !== undefined) next.v = cell.v;
    if (cell.t) next.t = cell.t;
    if (cell.fmt) next.fmt = cell.fmt;
    if (cell.style && Object.keys(cell.style).length) next.style = cell.style;
    if (cell.note) next.note = cell.note;
    if (next.f || next.v !== undefined || next.style || next.fmt) cells[key] = next;
  }

  return {
    id: sheet?.id || newSheetId(),
    name: sheet?.name || '시트1',
    dims: {
      rows: clampInt(sheet?.dims?.rows, DEFAULT_DIMS.rows, 1, 100000),
      cols: clampInt(sheet?.dims?.cols, DEFAULT_DIMS.cols, 1, 702),
    },
    frozen: {
      rows: clampInt(sheet?.frozen?.rows, 1, 0, 20),
      cols: clampInt(sheet?.frozen?.cols, 0, 0, 20),
    },
    colWidths: pickNumericMap(sheet?.colWidths),
    rowHeights: pickNumericMap(sheet?.rowHeights),
    merges: Array.isArray(sheet?.merges) ? sheet.merges : [],
    names: sheet?.names && typeof sheet.names === 'object' ? { ...sheet.names } : {},
    charts: normalizeCharts(sheet?.charts),
    cells,
  };
}

/**
 * Floating charts on a sheet, the way Excel places them over the grid.
 *
 * Geometry is stored in pixels relative to the sheet's top-left so the chart
 * survives column resizing, and the spec keeps a `range` rather than a copy of the
 * numbers — the chart is a view of the cells, not a snapshot.
 */
function normalizeCharts(charts) {
  if (!Array.isArray(charts)) return [];
  return charts.slice(0, 24).map((chart, i) => ({
    id: chart?.id || newBlockId(),
    x: clampInt(chart?.x, 80 + i * 24, 0, 20000),
    y: clampInt(chart?.y, 80 + i * 24, 0, 40000),
    w: clampInt(chart?.w, 460, 160, 2400),
    h: clampInt(chart?.h, 300, 120, 1600),
    spec: normalizeChartSpec(chart?.spec),
  }));
}

/** Smallest rectangle containing every non-empty cell. null when the sheet is empty. */
export function usedRange(cells) {
  let minCol = Infinity, minRow = Infinity, maxCol = -1, maxRow = -1;
  for (const ref of Object.keys(cells ?? {})) {
    const p = parseRef(ref);
    if (!p) continue;
    const cell = cells[ref];
    const empty = (cell.v === null || cell.v === undefined || cell.v === '') && !cell.f && !cell.style;
    if (empty) continue;
    minCol = Math.min(minCol, p.col);
    minRow = Math.min(minRow, p.row);
    maxCol = Math.max(maxCol, p.col);
    maxRow = Math.max(maxRow, p.row);
  }
  if (maxCol < 0) return null;
  return { minCol: 0, minRow: 0, maxCol, maxRow, firstCol: minCol, firstRow: minRow };
}

/**
 * Render the AI-readable markdown projection of a sheet.
 *
 * Column letters and row numbers are included deliberately: they let a model
 * reason in spreadsheet terms ("the total in D4") instead of guessing from a
 * headerless table. Formulas get their own section with both the expression and
 * the computed value, so a RAG chunk carries the intent *and* the answer.
 */
export function renderSheetMarkdown(sheet) {
  const s = sheet;
  const range = usedRange(s.cells);
  const lines = [];

  const rows = range ? Math.min(range.maxRow + 1, MD_ROW_LIMIT) : 0;
  const cols = range ? range.maxCol + 1 : 0;

  lines.push(`## ${s.name}`);
  lines.push('');

  if (!range) {
    lines.push('_빈 시트_');
  } else {
    const header = ['   ', ...Array.from({ length: cols }, (_, c) => indexToCol(c))];
    lines.push(`| ${header.join(' | ')} |`);
    lines.push(`|${header.map(() => '---').join('|')}|`);

    for (let r = 0; r < rows; r++) {
      const cellsInRow = [];
      for (let c = 0; c < cols; c++) {
        const cell = s.cells[toRef(c, r)];
        cellsInRow.push(mdCell(cell));
      }
      lines.push(`| **${r + 1}** | ${cellsInRow.join(' | ')} |`);
    }

    if (range.maxRow + 1 > MD_ROW_LIMIT) {
      lines.push('');
      lines.push(`_… ${range.maxRow + 1 - MD_ROW_LIMIT}개 행 생략 (전체 데이터는 \`${sheetJsonName(s.name)}\` 참조)_`);
    }
  }

  const formulas = Object.entries(s.cells)
    .filter(([, c]) => c.f)
    .sort(([a], [b]) => refOrder(a) - refOrder(b));

  if (formulas.length) {
    lines.push('');
    lines.push('### 수식');
    lines.push('');
    for (const [ref, cell] of formulas) {
      const shown = displayValue(cell) || '(빈 값)';
      lines.push(`- \`${ref}\` = \`${cell.f}\` → ${shown}`);
    }
  }

  const names = Object.entries(s.names ?? {});
  if (names.length) {
    lines.push('');
    lines.push('### 이름 있는 범위');
    lines.push('');
    for (const [name, target] of names) lines.push(`- \`${name}\` → \`${target}\``);
  }

  if (s.charts?.length) {
    lines.push('');
    lines.push('### 차트');
    lines.push('');
    for (const chart of s.charts) {
      const resolved = resolveChartSpec(chart.spec, s);
      lines.push(`- ${describeChart(resolved)}`);
      lines.push('');
      // The chart's numbers, not a picture of them: this is what a model reads.
      for (const row of chartToMarkdownTable(resolved).split('\n')) lines.push(`  ${row}`);
      lines.push('');
    }
  }

  const summary = summarizeColumns(s);
  if (summary.length) {
    const scope = summaryScope(s);
    lines.push('');
    lines.push('### 열 요약');
    lines.push('');
    if (scope) {
      const excluded = scope.excluded.length
        ? ` (집계 행 ${scope.excluded.map((r) => `${r}행`).join(', ')} 제외)`
        : '';
      lines.push(`집계 대상: ${scope.firstRow}–${scope.lastRow}행${excluded}`);
      lines.push('');
    }
    lines.push('| 열 | 머리글 | 유형 | 값 개수 | 합계 | 평균 | 최소 | 최대 |');
    lines.push('|---|---|---|---|---|---|---|---|');
    for (const col of summary) {
      lines.push(
        `| ${col.col} | ${escapePipes(col.header)} | ${col.type} | ${col.count} | ${col.sum ?? '—'} | ${col.avg ?? '—'} | ${col.min ?? '—'} | ${col.max ?? '—'} |`
      );
    }
  }

  const meta = {
    id: s.id,
    name: s.name,
    rows: range ? range.maxRow + 1 : 0,
    cols: range ? range.maxCol + 1 : 0,
    formulas: formulas.length,
  };
  return serializeFrontmatter(meta, lines.join('\n'));
}

function mdCell(cell) {
  if (!cell) return '';
  const text = escapePipes(displayValue(cell));
  if (!text) return '';
  return cell.style?.bold ? `**${text}**` : text;
}

function escapePipes(text) {
  return String(text ?? '').replace(/\|/g, '\\|').replace(/\n/g, ' ');
}

function refOrder(ref) {
  const p = parseRef(ref);
  return p ? p.row * 1000 + p.col : 0;
}

/**
 * Where the leading table ends: the row before the first completely blank row.
 *
 * Sheets routinely put unrelated blocks below a blank row (an analysis section, a
 * second table). Summing straight down a column across that boundary produces
 * numbers that match nothing in the sheet, which is worse than no summary at all
 * for a model reading the digest.
 */
export function tableExtent(cells, maxRow, maxCol) {
  for (let r = 1; r <= maxRow; r++) {
    let blank = true;
    for (let c = 0; c <= maxCol; c++) {
      const cell = cells[toRef(c, r)];
      if (cell && cell.v !== null && cell.v !== undefined && cell.v !== '') {
        blank = false;
        break;
      }
    }
    if (blank) return r - 1;
  }
  return maxRow;
}

const TOTAL_LABEL = /^\s*(합계|총계|소계|누계|계|total|subtotal|sum)\s*$/i;

/**
 * A row that aggregates the rows above it, which must be excluded from column
 * sums or every total gets counted twice.
 *
 * Detected two ways: by its label, and structurally by a cell whose formula
 * aggregates a range inside its own column — the latter catches unlabelled
 * total rows in any language.
 */
function isAggregateRow(cells, row, maxCol) {
  const label = displayValue(cells[toRef(0, row)]);
  if (TOTAL_LABEL.test(label)) return true;

  for (let c = 0; c <= maxCol; c++) {
    const cell = cells[toRef(c, row)];
    if (!cell?.f) continue;
    const col = indexToCol(c);
    const selfColumn = new RegExp(`\\b(SUM|AVERAGE|COUNT|MAX|MIN)\\(\\$?${col}\\$?\\d+:\\$?${col}\\$?\\d+\\)`, 'i');
    if (selfColumn.test(cell.f)) return true;
  }
  return false;
}

/**
 * Per-column statistics appended to the markdown.
 *
 * A model asking "what is in column C?" gets an answer without scanning rows —
 * and the aggregates are restricted to the leading table's data rows so they
 * reconcile with the sheet's own total row instead of contradicting it.
 */
export function summarizeColumns(sheet) {
  const range = usedRange(sheet.cells);
  if (!range) return [];

  const lastRow = tableExtent(sheet.cells, range.maxRow, range.maxCol);
  const dataRows = [];
  for (let r = 1; r <= lastRow; r++) {
    if (!isAggregateRow(sheet.cells, r, range.maxCol)) dataRows.push(r);
  }

  const out = [];
  for (let c = 0; c <= range.maxCol; c++) {
    const col = indexToCol(c);
    const header = displayValue(sheet.cells[toRef(c, 0)]) || '';
    let count = 0;
    let numeric = 0;
    let sum = 0;
    let min = Infinity;
    let max = -Infinity;

    for (const r of dataRows) {
      const cell = sheet.cells[toRef(c, r)];
      if (!cell || cell.v === null || cell.v === undefined || cell.v === '') continue;
      count++;
      if (typeof cell.v === 'number' && cell.t !== 'd' && cell.t !== 'e') {
        numeric++;
        sum += cell.v;
        min = Math.min(min, cell.v);
        max = Math.max(max, cell.v);
      }
    }
    if (count === 0 && !header) continue;

    const isNumeric = count > 0 && numeric / count >= 0.6;
    const fmt = sheet.cells[toRef(c, dataRows[0] ?? 1)]?.fmt;
    out.push({
      col,
      header: header || '(머리글 없음)',
      type: isNumeric ? '숫자' : count ? '텍스트' : '빈 열',
      count,
      sum: isNumeric ? formatStat(sum, fmt) : null,
      avg: isNumeric && numeric ? formatStat(sum / numeric, fmt) : null,
      min: isNumeric && numeric ? formatStat(min, fmt) : null,
      max: isNumeric && numeric ? formatStat(max, fmt) : null,
    });
  }
  return out;
}

/** Rows the summary is based on, so the markdown can say so explicitly. */
export function summaryScope(sheet) {
  const range = usedRange(sheet.cells);
  if (!range) return null;
  const lastRow = tableExtent(sheet.cells, range.maxRow, range.maxCol);
  const excluded = [];
  for (let r = 1; r <= lastRow; r++) {
    if (isAggregateRow(sheet.cells, r, range.maxCol)) excluded.push(r + 1);
  }
  return { firstRow: 2, lastRow: lastRow + 1, excluded, truncated: lastRow < range.maxRow };
}

function formatStat(n, fmt) {
  if (fmt && /%/.test(fmt)) return `${(Math.round(n * 1000) / 10).toLocaleString('en-US')}%`;
  const rounded = Math.round(n * 100) / 100;
  return rounded.toLocaleString('en-US');
}

/**
 * Import a markdown table into cells.
 *
 * Recognises the projection format this module emits (leading blank corner +
 * column letters, bolded row numbers) and also plain markdown tables, which land
 * at A1 with the header row intact.
 */
export function importMarkdownTable(md) {
  const { body } = parseFrontmatter(md);
  const lines = String(body).split(/\r?\n/);
  const tableLines = [];
  let inTable = false;

  for (const line of lines) {
    if (/^[ \t]*\|/.test(line)) {
      inTable = true;
      tableLines.push(line.trim());
      continue;
    }
    if (inTable) break;
  }
  if (tableLines.length < 2) return {};

  const rows = tableLines
    .filter((l) => !/^\|[\s:|-]+\|$/.test(l))
    .map((l) => l.replace(/^\|/, '').replace(/\|$/, '').split('|').map((c) => c.trim()));
  if (!rows.length) return {};

  const header = rows[0];
  const looksProjected =
    header[0] === '' && header.slice(1).every((h) => /^[A-Z]{1,3}$/.test(h)) && header.length > 1;

  const cells = {};
  const dataRows = looksProjected ? rows.slice(1) : rows;

  dataRows.forEach((row, rIdx) => {
    const cols = looksProjected ? row.slice(1) : row;
    let rowIndex = rIdx;
    if (looksProjected) {
      const label = row[0].replace(/\*/g, '').trim();
      const parsed = Number(label);
      if (Number.isFinite(parsed) && parsed > 0) rowIndex = parsed - 1;
    }
    cols.forEach((raw, cIdx) => {
      const bold = /^\*\*.*\*\*$/.test(raw);
      const text = raw.replace(/^\*\*|\*\*$/g, '').replace(/\\\|/g, '|').trim();
      if (!text) return;
      const cell = parseCellInput(text);
      if (!cell) return;
      if (bold) cell.style = { bold: true };
      cells[toRef(cIdx, rowIndex)] = cell;
    });
  });

  return cells;
}

function stripEmpty(cells) {
  const out = {};
  for (const [ref, cell] of Object.entries(cells)) {
    const isEmpty =
      !cell.f &&
      (cell.v === null || cell.v === undefined || cell.v === '') &&
      !cell.fmt &&
      !(cell.style && Object.keys(cell.style).length) &&
      !cell.note;
    if (!isEmpty) out[ref] = cell;
  }
  return out;
}

function clampInt(v, fallback, min, max) {
  const n = Number(v);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, Math.round(n)));
}

function pickNumericMap(map) {
  const out = {};
  for (const [k, v] of Object.entries(map ?? {})) {
    const n = Number(v);
    if (Number.isFinite(n) && n > 0) out[k] = Math.round(n);
  }
  return out;
}

function sheetJsonName(name) {
  return `${name}.cells.json`;
}

export function makeSheet({ name = '시트1', withSample = false } = {}) {
  const cells = {};
  if (withSample) {
    const headers = ['항목', '1분기', '2분기', '합계'];
    headers.forEach((h, i) => {
      cells[toRef(i, 0)] = { v: h, t: 's', style: { bold: true, bg: '#f1f5f9' } };
    });
    const data = [
      ['제품 A', 1200, 1350],
      ['제품 B', 980, 1120],
      ['제품 C', 640, 890],
    ];
    data.forEach(([label, q1, q2], i) => {
      const r = i + 1;
      cells[toRef(0, r)] = { v: label, t: 's' };
      cells[toRef(1, r)] = { v: q1, t: 'n', fmt: '#,##0' };
      cells[toRef(2, r)] = { v: q2, t: 'n', fmt: '#,##0' };
      cells[toRef(3, r)] = { f: `=B${r + 1}+C${r + 1}`, t: 'n', fmt: '#,##0' };
    });
    const totalRow = data.length + 1;
    cells[toRef(0, totalRow)] = { v: '총계', t: 's', style: { bold: true } };
    for (let c = 1; c <= 3; c++) {
      const col = indexToCol(c);
      cells[toRef(c, totalRow)] = {
        f: `=SUM(${col}2:${col}${totalRow})`,
        t: 'n',
        fmt: '#,##0',
        style: { bold: true, bg: '#f8fafc' },
      };
    }
  }
  return normalizeSheet({ name, cells, frozen: { rows: 1, cols: 1 }, charts: [] });
}
