/**
 * The document core, as the editors see it.
 *
 * Every function here is the Rust implementation in `crates/`, compiled to
 * WebAssembly. There is no second copy of the formula engine or the save format
 * in JavaScript — what runs in this editor is byte-for-byte the code that writes
 * the files, so the `{ } 저장 포맷` panel cannot disagree with the disk.
 *
 * The wasm module is initialised once, before React mounts (see `main.jsx`), so
 * every call below is an ordinary synchronous function. That matters: a cell
 * edit recalculates its dependents inside the same event handler.
 */
import init, * as wasm from './pkg/ai_studio_wasm.js';

/**
 * Where the wasm binary lives, relative to this module.
 *
 * `new URL(…, import.meta.url)` is the form bundlers understand as an asset
 * reference. Deliberately lazy: a bundle built without module metadata — the
 * jsdom harness, which supplies the bytes itself — would otherwise throw on
 * `new URL(relative, '')` while merely importing this file.
 */
const wasmUrl = () => new URL('./pkg/ai_studio_wasm_bg.wasm', import.meta.url);

let ready = false;

/**
 * Load the wasm module and mirror its constants. Call once, and await it before
 * rendering; every other export in this file is synchronous afterwards.
 *
 * @param source optional bytes or URL, for environments without `fetch`.
 */
export async function initCore(source) {
  if (ready) return;
  await init({ module_or_path: source ?? wasmUrl() });
  hydrateConstants();
  ready = true;
}

export const coreReady = () => ready;

function assertReady() {
  if (!ready) throw new Error('코어가 초기화되지 않았습니다 — initCore()를 먼저 기다리세요');
}

/* ------------------------------------------------------------------- refs */

export const indexToCol = (index) => wasm.indexToCol(index);
export const colToIndex = (col) => wasm.colToIndex(col);
export const toRef = (col, row) => wasm.toRef(col, row);
export const parseRef = (ref) => wasm.parseRef(String(ref ?? ''));
export const parseRange = (range) => wasm.parseRange(String(range ?? ''));
export const expandRange = (range) => wasm.expandRange(String(range ?? ''));
export const shiftRef = (ref, dc, dr) => wasm.shiftRef(String(ref ?? ''), dc, dr);
export const shiftFormula = (formula, dc, dr) => wasm.shiftFormula(String(formula ?? ''), dc, dr);
export const adjustRefs = (formula, axis, at, delta) =>
  wasm.adjustRefs(String(formula ?? ''), axis, at, delta);

/* --------------------------------------------------------------- formulas */

/**
 * Recalculate one sheet.
 *
 * `others` are the workbook's remaining sheets; without them a formula that
 * reaches `=요약!B4` cannot be resolved, and the core then leaves the value the
 * file already had rather than writing `#REF!` over it.
 */
export function recalcSheet(sheet, others = []) {
  assertReady();
  return wasm.recalcSheet(
    { cells: sheet?.cells ?? {}, names: sheet?.names ?? {}, name: sheet?.name ?? '' },
    others.map((s) => ({ cells: s?.cells ?? {}, names: {}, name: s?.name ?? '' }))
  );
}

export const parseCellInput = (raw) => wasm.parseCellInput(raw === null || raw === undefined ? '' : String(raw));
export const displayValue = (cell) => wasm.displayValue(cell ?? null);
export const editValue = (cell) => wasm.editValue(cell ?? null);
export const dependencies = (formula) => wasm.dependencies(String(formula ?? ''));
export const applyNumFmt = (value, fmt) => wasm.applyNumFmt(value ?? null, String(fmt ?? ''));



/* --------------------------------------------------------------- geometry */

export const DEFAULT_CANVAS = { w: 1280, h: 720, bg: '#ffffff' };

export const positionPhrase = (box, canvas = DEFAULT_CANVAS) =>
  box ? wasm.positionPhrase(box, canvas) : '';
export const autoLayout = (index, total, canvas = DEFAULT_CANVAS) =>
  wasm.autoLayout(index, total, canvas);
export const clampBox = (box, canvas = DEFAULT_CANVAS) => wasm.clampBox(box, canvas);

/** Reading order: top-to-bottom, then left-to-right, with a row tolerance. */
export function readingOrder(blocks, rowTolerance = 40) {
  return [...blocks].sort((a, b) => {
    const dy = (a.y ?? 0) - (b.y ?? 0);
    if (Math.abs(dy) > rowTolerance) return dy;
    return (a.x ?? 0) - (b.x ?? 0);
  });
}

/* ------------------------------------------------------------------- page */

/** A page's real size in px. Explicit dimensions win over the paper name. */
export const pageDims = (page) => wasm.pageDims(page ?? {});
/** The same page at a new size, named if the size has a name. */
export const pageResize = (page, w, h) => wasm.pageResize(page ?? {}, w, h);
/** A header or footer's three slots, with the page tokens filled in. */
export const resolveRunning = (running, page, pages, today = '') =>
  wasm.resolveRunning(running ?? {}, page, pages, today);

/* --------------------------------------------------------------- markdown */

export const headingLevel = (md) => wasm.headingLevel(String(md ?? ''));
export const plainText = (md) => wasm.plainText(String(md ?? ''));
export const countWords = (text) => wasm.countWords(String(text ?? ''));
export const inferKind = (md) => wasm.inferKind(String(md ?? ''));
export const blockLabel = (md, max) => wasm.blockLabel(String(md ?? ''), max);
export const splitMarkdownBlocks = (md) => wasm.splitMarkdownBlocks(String(md ?? ''));

/* ----------------------------------------------------------------- charts */

export const normalizeChartSpec = (spec) => wasm.normalizeChartSpec(spec ?? null);
export const parseChartBlock = (md) => wasm.parseChartBlock(String(md ?? ''));
export const serializeChartBlock = (spec) => wasm.serializeChartBlock(spec ?? {});
export const resolveChartSpec = (spec, sheet) => wasm.resolveChartSpec(spec ?? {}, sheet ?? null);
export const chartToMarkdownTable = (spec) => wasm.chartToMarkdownTable(spec ?? {});
export const describeChart = (spec) => wasm.describeChart(spec ?? {});

/* ----------------------------------------------------------------- shapes */

/** The shape gallery, grouped as Office's picker groups it. */
export const shapeGallery = () => wasm.shapeGallery();

export const makeShape = (preset, box = {}) => wasm.makeShape(preset, box);
export const makeTable = (columns, rows, box = {}) => wasm.makeTable(columns, rows, box);
export const blankTable = (columns, rows) => wasm.blankTable(columns, rows);

/* ------------------------------------------------------------- table edits */

/**
 * One structural edit to a table, applied by the Rust core.
 *
 * Merges have to move when a row is inserted or a column deleted, and getting
 * that wrong draws a table with holes. It is the same class of problem as
 * adjusting a formula's references, so it lives with the format rather than in
 * the editor.
 *
 * `op` is one of `setCell` · `insert` · `delete` · `merge` · `split` · `format`.
 */
export const tableEdit = (md, spec, op, args = {}) => wasm.tableEdit(md, spec ?? null, op, args);

/** A table's shape and its cells, without editing it. */
export const tableInfo = (md, spec) => wasm.tableInfo(md, spec ?? null);

/** Where the cursor goes for a navigation key. `addRow` marks Tab at the end. */
export const tableMove = (md, spec, col, row, key, backwards = false) =>
  wasm.tableMove(md, spec ?? null, col, row, key, backwards);

/* ----------------------------------------------------------- new documents */

export const makeSlide = (layout = 'title-content', { title, index = 1 } = {}) =>
  wasm.makeSlide(layout, title ?? undefined, index);
export const makeSection = ({ name = '새 섹션', heading = true } = {}) =>
  wasm.makeSection(name, heading);
export const makeSheet = ({ name = '시트1', withSample = false } = {}) =>
  wasm.makeSheet(name, withSample);

export const newBlockId = () => wasm.newBlockId();
export const slugify = (title) => wasm.slugify(String(title ?? ''));
export const usedRange = (cells) => wasm.usedRange(cells ?? {});

/* --------------------------------------------------------------- save form */

export const writeSlide = (slide) => wasm.writeSlide(slide);
export const writeSection = (section) => wasm.writeSection(section);
export const writeSheet = (sheet) => wasm.writeSheet(sheet);
export const buildDigest = (project) => wasm.buildDigest(project);

/* -------------------------------------------------------------- constants */

/**
 * Values the Rust core owns. Filled in by `initCore` so the editors can use
 * them as plain constants instead of calling across the boundary in a render.
 */
export const CHART_TYPES = [];
export const CHART_TYPE_LABELS = {};
export const SLIDE_LAYOUTS = [];
export const PAGE_SIZES = {};
/** What the page-size dropdown calls a size that is not a named paper. */
export const CUSTOM_PAPER = { label: '사용자 지정' };
export const CHART_PALETTE = { light: [], dark: [] };
/** Sorted function names, for the formula bar's autocomplete. */
export const FUNCTION_NAMES = [];

/**
 * Numbers the Rust core owns.
 *
 * A mutable object rather than `export let` bindings, so a consumer that
 * destructures at module scope still cannot capture a pre-init value.
 */
export const LIMITS = {
  maxSeries: 8,
  cellPx: 15,
  colWidth: 104,
  rowHeight: 28,
  dims: { rows: 200, cols: 26 },
};

function hydrateConstants() {
  for (const { type, label } of wasm.chartTypes()) {
    CHART_TYPES.push(type);
    CHART_TYPE_LABELS[type] = label;
  }
  SLIDE_LAYOUTS.push(...wasm.slideLayouts());
  Object.assign(PAGE_SIZES, wasm.pageSizes());
  CUSTOM_PAPER.label = wasm.customPaperLabel();
  FUNCTION_NAMES.push(...wasm.functionNames());

  const palette = wasm.chartPalette();
  CHART_PALETTE.light = palette.light;
  CHART_PALETTE.dark = palette.dark;
  LIMITS.maxSeries = palette.maxSeries;

  const grid = wasm.gridDefaults();
  LIMITS.cellPx = grid.cellPx;
  LIMITS.colWidth = grid.colWidth;
  LIMITS.rowHeight = grid.rowHeight;
  LIMITS.dims = grid.dims;
}
