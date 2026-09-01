import test from 'node:test';
import assert from 'node:assert/strict';

import { loadCore } from './core.mjs';

// The grid operations call into the wasm core synchronously, so it has to be
// resolved before the module graph below is evaluated.
await loadCore();
const { toRef, displayValue } = await import('../src/core/index.js');
const {
  setCellInput, patchCells, clearRange, structuralEdit,
  fillTarget, fillRange, mergeSelection, unmergeSelection, mergeCovering,
  applyBorders, BORDER_PRESETS, setColWidth, setRowHeight, autoFitColumn,
  resolveTarget, findCells, replaceInCells,
  rangeToTsv, rangeToFormulaTsv, pasteTsv, normalizeRange, selectionStats,
} = await import('../src/grid/gridOps.js');

const sheetOf = (cells, extra = {}) => ({
  id: 'sh', name: 's',
  dims: { rows: 200, cols: 26 },
  frozen: { rows: 1, cols: 0 },
  colWidths: {}, rowHeights: {}, merges: [], names: {},
  cells, ...extra,
});

const at = (sheet, ref) => sheet.cells[ref];
const shown = (sheet, ref) => displayValue(sheet.cells[ref]);

/* -------------------------------------------------------------- cell input */

test('typing into a cell parses and recalculates dependents', () => {
  let sheet = sheetOf({ A1: { v: 1, t: 'n' }, A2: { v: 2, t: 'n' }, A3: { f: '=SUM(A1:A2)' } });
  sheet = setCellInput(sheet, 0, 0, '10');
  assert.equal(at(sheet, 'A1').v, 10);
  assert.equal(at(sheet, 'A3').v, 12, 'the dependent formula was recalculated');
});

test('clearing a cell keeps its styling, as Delete does in Excel', () => {
  let sheet = sheetOf({ A1: { v: 5, t: 'n', style: { bold: true } } });
  sheet = setCellInput(sheet, 0, 0, '');
  assert.equal(shown(sheet, 'A1'), '', 'the value is gone');
  assert.equal(at(sheet, 'A1').style.bold, true, 'the styling stayed');
});

test('an existing number format survives retyping a value', () => {
  let sheet = sheetOf({ A1: { v: 1000, t: 'n', fmt: '₩#,##0' } });
  sheet = setCellInput(sheet, 0, 0, '2500');
  assert.equal(at(sheet, 'A1').fmt, '₩#,##0');
  assert.equal(shown(sheet, 'A1'), '₩2,500');
});

test('patchCells removes a style property when passed null', () => {
  let sheet = sheetOf({ A1: { v: 1, style: { bold: true, bg: '#fff' } } });
  sheet = patchCells(sheet, ['A1'], { style: { bold: null } });
  assert.equal(at(sheet, 'A1').style.bold, undefined);
  assert.equal(at(sheet, 'A1').style.bg, '#fff');
});

/* ------------------------------------------------------------ fill handle */

test('fillTarget extends along the axis the pointer moved furthest', () => {
  const source = { r1: 1, c1: 1, r2: 1, c2: 1 };
  assert.equal(fillTarget(source, 5, 1).dir, 'down');
  assert.equal(fillTarget(source, 1, 5).dir, 'right');
  assert.equal(fillTarget(source, 0, 1).dir, 'up');
  assert.equal(fillTarget(source, 1, 0).dir, 'left');
  assert.equal(fillTarget(source, 1, 1), null, 'no movement means no fill');
  assert.deepEqual(
    { r1: 1, r2: 5 },
    { r1: fillTarget(source, 5, 1).r1, r2: fillTarget(source, 5, 1).r2 },
    'the source stays inside the target'
  );
});

test('dragging a single number copies it', () => {
  const source = { r1: 0, c1: 0, r2: 0, c2: 0 };
  let sheet = sheetOf({ A1: { v: 7, t: 'n' } });
  sheet = fillRange(sheet, source, fillTarget(source, 3, 0));
  assert.deepEqual([at(sheet, 'A2').v, at(sheet, 'A3').v, at(sheet, 'A4').v], [7, 7, 7]);
});

test('dragging two numbers extrapolates the series', () => {
  const source = { r1: 0, c1: 0, r2: 1, c2: 0 };
  let sheet = sheetOf({ A1: { v: 10, t: 'n' }, A2: { v: 20, t: 'n' } });
  sheet = fillRange(sheet, source, fillTarget(source, 4, 0));
  assert.deepEqual(
    [at(sheet, 'A3').v, at(sheet, 'A4').v, at(sheet, 'A5').v],
    [30, 40, 50]
  );
});

test('a descending series extrapolates downward', () => {
  const source = { r1: 0, c1: 0, r2: 1, c2: 0 };
  let sheet = sheetOf({ A1: { v: 100, t: 'n' }, A2: { v: 90, t: 'n' } });
  sheet = fillRange(sheet, source, fillTarget(source, 3, 0));
  assert.deepEqual([at(sheet, 'A3').v, at(sheet, 'A4').v], [80, 70]);
});

test('filling a formula shifts its relative references', () => {
  const source = { r1: 0, c1: 3, r2: 0, c2: 3 };
  let sheet = sheetOf({
    A1: { v: 1 }, B1: { v: 2 }, D1: { f: '=A1+B1' },
    A2: { v: 10 }, B2: { v: 20 },
    A3: { v: 100 }, B3: { v: 200 },
  });
  sheet = fillRange(sheet, source, fillTarget(source, 2, 3));
  assert.equal(at(sheet, 'D2').f, '=A2+B2');
  assert.equal(at(sheet, 'D2').v, 30, 'and the filled formula is computed');
  assert.equal(at(sheet, 'D3').f, '=A3+B3');
  assert.equal(at(sheet, 'D3').v, 300);
});

test('filling a formula keeps absolute references pinned', () => {
  const source = { r1: 0, c1: 1, r2: 0, c2: 1 };
  let sheet = sheetOf({ A1: { v: 5 }, A2: { v: 6 }, C1: { v: 2 }, B1: { f: '=A1*$C$1' } });
  sheet = fillRange(sheet, source, fillTarget(source, 1, 1));
  assert.equal(at(sheet, 'B2').f, '=A2*$C$1');
  assert.equal(at(sheet, 'B2').v, 12);
});

test('text ending in digits increments', () => {
  const source = { r1: 0, c1: 0, r2: 0, c2: 0 };
  let sheet = sheetOf({ A1: { v: '항목 1', t: 's' } });
  sheet = fillRange(sheet, source, fillTarget(source, 2, 0));
  assert.equal(at(sheet, 'A2').v, '항목 2');
  assert.equal(at(sheet, 'A3').v, '항목 3');
});

test('zero-padded text series keeps its width', () => {
  const source = { r1: 0, c1: 0, r2: 0, c2: 0 };
  let sheet = sheetOf({ A1: { v: 'ID-008', t: 's' } });
  sheet = fillRange(sheet, source, fillTarget(source, 2, 0));
  assert.equal(at(sheet, 'A2').v, 'ID-009');
  assert.equal(at(sheet, 'A3').v, 'ID-010');
});

test('plain text without digits repeats', () => {
  const source = { r1: 0, c1: 0, r2: 0, c2: 0 };
  let sheet = sheetOf({ A1: { v: '서울', t: 's' } });
  sheet = fillRange(sheet, source, fillTarget(source, 2, 0));
  assert.equal(at(sheet, 'A2').v, '서울');
  assert.equal(at(sheet, 'A3').v, '서울');
});

test('filling right works across columns', () => {
  const source = { r1: 0, c1: 0, r2: 0, c2: 1 };
  let sheet = sheetOf({ A1: { v: 1, t: 'n' }, B1: { v: 2, t: 'n' } });
  sheet = fillRange(sheet, source, fillTarget(source, 0, 4));
  assert.deepEqual([at(sheet, 'C1').v, at(sheet, 'D1').v, at(sheet, 'E1').v], [3, 4, 5]);
});

test('filling upward extrapolates backwards', () => {
  const source = { r1: 3, c1: 0, r2: 4, c2: 0 };
  let sheet = sheetOf({ A4: { v: 30, t: 'n' }, A5: { v: 40, t: 'n' } });
  sheet = fillRange(sheet, source, fillTarget(source, 1, 0));
  assert.equal(at(sheet, 'A3').v, 20, 'A3 continues the series upward');
  assert.equal(at(sheet, 'A2').v, 10);
});

test('filling copies the source format and style', () => {
  const source = { r1: 0, c1: 0, r2: 0, c2: 0 };
  let sheet = sheetOf({ A1: { v: 0.25, t: 'n', fmt: '0.0%', style: { bold: true } } });
  sheet = fillRange(sheet, source, fillTarget(source, 1, 0));
  assert.equal(at(sheet, 'A2').fmt, '0.0%');
  assert.equal(at(sheet, 'A2').style.bold, true);
});

test('a fill that adds nothing leaves the sheet alone', () => {
  const source = { r1: 0, c1: 0, r2: 0, c2: 0 };
  const sheet = sheetOf({ A1: { v: 1 } });
  assert.equal(fillRange(sheet, source, { ...source, dir: 'down' }), sheet);
});

/* ----------------------------------------------------------------- merges */

test('merging keeps the top-left value and drops the rest', () => {
  let sheet = sheetOf({
    A1: { v: '제목', t: 's' },
    B1: { v: '버릴 값', t: 's' },
    A2: { v: 'x', t: 's' },
  });
  sheet = mergeSelection(sheet, { r1: 0, c1: 0, r2: 1, c2: 1 });
  assert.deepEqual(sheet.merges, ['A1:B2']);
  assert.equal(at(sheet, 'A1').v, '제목');
  assert.equal(at(sheet, 'B1'), undefined);
  assert.equal(at(sheet, 'A2'), undefined);
});

test('mergeCovering finds the rectangle over any covered cell', () => {
  const sheet = sheetOf({}, { merges: ['B2:C3'] });
  assert.equal(mergeCovering(sheet, 1, 1).spec, 'B2:C3');
  assert.equal(mergeCovering(sheet, 2, 2).spec, 'B2:C3');
  assert.equal(mergeCovering(sheet, 0, 0), null);
});

test('a new merge replaces any merge it overlaps', () => {
  let sheet = sheetOf({}, { merges: ['A1:B1'] });
  sheet = mergeSelection(sheet, { r1: 0, c1: 0, r2: 0, c2: 3 });
  assert.deepEqual(sheet.merges, ['A1:D1'], 'merges never intersect');
});

test('unmerge removes merges touching the selection', () => {
  let sheet = sheetOf({}, { merges: ['A1:B2', 'D4:E5'] });
  sheet = unmergeSelection(sheet, { r1: 0, c1: 0, r2: 0, c2: 0 });
  assert.deepEqual(sheet.merges, ['D4:E5']);
});

test('merging a single cell is a no-op', () => {
  const sheet = sheetOf({ A1: { v: 1 } });
  assert.equal(mergeSelection(sheet, { r1: 0, c1: 0, r2: 0, c2: 0 }), sheet);
});

/* ---------------------------------------------------------------- borders */

test('the all-borders preset sets every edge', () => {
  const sheet = applyBorders(sheetOf({ A1: { v: 1 } }), { r1: 0, c1: 0, r2: 0, c2: 0 }, BORDER_PRESETS.all);
  assert.deepEqual(at(sheet, 'A1').style.border, { t: true, r: true, b: true, l: true });
});

test('the outline preset only draws the perimeter', () => {
  const range = { r1: 0, c1: 0, r2: 1, c2: 1 };
  const sheet = applyBorders(sheetOf({}), range, 'outline');
  assert.deepEqual(at(sheet, 'A1').style.border, { t: true, l: true });
  assert.deepEqual(at(sheet, 'B1').style.border, { t: true, r: true });
  assert.deepEqual(at(sheet, 'A2').style.border, { b: true, l: true });
  assert.deepEqual(at(sheet, 'B2').style.border, { r: true, b: true });
});

test('clearing borders drops the property and empties the cell if nothing is left', () => {
  let sheet = applyBorders(sheetOf({}), { r1: 0, c1: 0, r2: 0, c2: 0 }, BORDER_PRESETS.all);
  assert.ok(at(sheet, 'A1'));
  sheet = applyBorders(sheet, { r1: 0, c1: 0, r2: 0, c2: 0 }, null);
  assert.equal(at(sheet, 'A1'), undefined, 'a border-only cell is removed again');
});

/* ------------------------------------------------------------------ sizes */

test('column widths are stored by letter and cleared at the default', () => {
  let sheet = setColWidth(sheetOf({}), 2, 180);
  assert.equal(sheet.colWidths.C, 180);
  sheet = setColWidth(sheet, 2, 104);
  assert.equal(sheet.colWidths.C, undefined, 'the default is not persisted');
});

test('row heights are stored by 1-based number', () => {
  const sheet = setRowHeight(sheetOf({}), 4, 44);
  assert.equal(sheet.rowHeights['5'], 44);
});

test('autoFitColumn widens for longer content and counts CJK as double', () => {
  const short = autoFitColumn(sheetOf({ A1: { v: 'ab', t: 's' } }), 0, 0);
  const long = autoFitColumn(sheetOf({ A1: { v: 'abcdefghijklmnop', t: 's' } }), 0, 0);
  const cjk = autoFitColumn(sheetOf({ A1: { v: '가나다라마바사아', t: 's' } }), 0, 0);
  assert.ok(long > short, `${long} > ${short}`);
  assert.ok(cjk > long / 2, 'CJK is not treated as narrow');
});

/* --------------------------------------------------------- go to and find */

test('the name box resolves refs, ranges and named ranges', () => {
  const sheet = sheetOf({}, { names: { 매출: 'B2:D4' } });
  assert.deepEqual(resolveTarget(sheet, 'C3'), { row: 2, col: 2, row2: 2, col2: 2 });
  assert.deepEqual(resolveTarget(sheet, 'A1:B2'), { row: 0, col: 0, row2: 1, col2: 1 });
  assert.deepEqual(resolveTarget(sheet, '매출'), { row: 1, col: 1, row2: 3, col2: 3 });
  assert.equal(resolveTarget(sheet, '없는이름'), null);
  assert.equal(resolveTarget(sheet, ''), null);
});

test('find returns hits in reading order', () => {
  const sheet = sheetOf({
    B2: { v: '서울 지점', t: 's' },
    A1: { v: '서울', t: 's' },
    C1: { v: '부산', t: 's' },
  });
  assert.deepEqual(findCells(sheet, '서울').map((h) => h.ref), ['A1', 'B2']);
  assert.deepEqual(findCells(sheet, '부산').map((h) => h.ref), ['C1']);
  assert.deepEqual(findCells(sheet, '없음'), []);
});

test('find can search formulas instead of displayed values', () => {
  const sheet = sheetOf({ A1: { v: 1 }, A2: { f: '=SUM(A1:A1)', v: 1, t: 'n' } });
  assert.deepEqual(findCells(sheet, 'SUM', { inFormulas: true }).map((h) => h.ref), ['A2']);
  assert.deepEqual(findCells(sheet, 'SUM').map((h) => h.ref), [], 'values do not contain SUM');
});

test('find is case-insensitive unless asked otherwise', () => {
  const sheet = sheetOf({ A1: { v: 'Seoul', t: 's' } });
  assert.equal(findCells(sheet, 'seoul').length, 1);
  assert.equal(findCells(sheet, 'seoul', { matchCase: true }).length, 0);
});

test('replace rewrites values and keeps formulas as formulas', () => {
  let sheet = sheetOf({
    A1: { v: '서울 지점', t: 's' },
    A2: { f: '=CONCAT("서울","!")', v: '서울!', t: 's' },
  });
  const hits = findCells(sheet, '서울', { inFormulas: true });
  sheet = replaceInCells(sheet, hits, '서울', '부산');
  assert.equal(at(sheet, 'A1').v, '부산 지점');
  assert.equal(at(sheet, 'A2').f, '=CONCAT("부산","!")');
  assert.equal(at(sheet, 'A2').v, '부산!', 'and it was recalculated');
});

/* ------------------------------------------------------ structural edits */

test('inserting a row moves cells and rewrites formulas', () => {
  let sheet = sheetOf({
    A1: { v: 1 }, A2: { v: 2 },
    B1: { f: '=SUM(A1:A2)' },
  });
  sheet = structuralEdit(sheet, 'row', 1, 1);
  assert.equal(at(sheet, 'A3').v, 2, 'A2 moved down to A3');
  assert.equal(at(sheet, 'A2'), undefined, 'and left a gap');
  assert.equal(at(sheet, 'B1').f, '=SUM(A1:A3)', 'the range grew');
  assert.equal(at(sheet, 'B1').v, 3);
});

test('deleting a row removes it and shifts the rest up', () => {
  let sheet = sheetOf({ A1: { v: 1 }, A2: { v: 2 }, A3: { v: 3 } });
  sheet = structuralEdit(sheet, 'row', 1, -1);
  assert.equal(at(sheet, 'A2').v, 3);
  assert.equal(at(sheet, 'A3'), undefined);
});

test('inserting a column shifts stored widths', () => {
  let sheet = setColWidth(sheetOf({}), 1, 200);
  sheet = structuralEdit(sheet, 'col', 0, 1);
  assert.equal(sheet.colWidths.C, 200, 'the B width moved to C');
});

/* -------------------------------------------------------------- clipboard */

test('copy produces TSV of displayed values, and formulas on request', () => {
  const sheet = sheetOf({
    A1: { v: '항목', t: 's' }, B1: { v: 1000, t: 'n', fmt: '#,##0' },
    A2: { v: '합', t: 's' }, B2: { f: '=B1*2', v: 2000, t: 'n', fmt: '#,##0' },
  });
  const range = { r1: 0, c1: 0, r2: 1, c2: 1 };
  assert.equal(rangeToTsv(sheet, range), '항목\t1,000\n합\t2,000');
  assert.equal(rangeToFormulaTsv(sheet, range), '항목\t1000\n합\t=B1*2');
});

test('paste reads TSV into cells and computes pasted formulas', () => {
  let sheet = sheetOf({});
  sheet = pasteTsv(sheet, '10\t20\n=A1+B1\t30', 0, 0);
  assert.equal(at(sheet, 'B1').v, 20);
  assert.equal(at(sheet, 'A2').f, '=A1+B1');
  assert.equal(at(sheet, 'A2').v, 30);
});

/* ------------------------------------------------------------- selection */

test('normalizeRange orders the corners', () => {
  assert.deepEqual(normalizeRange({ row: 5, col: 5, row2: 1, col2: 2 }), { r1: 1, r2: 5, c1: 2, c2: 5 });
  assert.deepEqual(normalizeRange({ row: 2, col: 3 }), { r1: 2, r2: 2, c1: 3, c2: 3 });
});

test('selectionStats ignores blanks and text', () => {
  const sheet = sheetOf({ A1: { v: 10 }, A2: { v: 'x', t: 's' }, A3: { v: 20 } });
  const stats = selectionStats(sheet, { r1: 0, c1: 0, r2: 3, c2: 0 });
  assert.equal(stats.count, 3);
  assert.equal(stats.numeric, 2);
  assert.equal(stats.sum, 30);
  assert.equal(stats.avg, 15);
});

test('clearRange can wipe styles too', () => {
  let sheet = sheetOf({ A1: { v: 1, style: { bold: true } } });
  sheet = clearRange(sheet, ['A1'], { keepStyle: false });
  assert.equal(at(sheet, 'A1'), undefined);
});
