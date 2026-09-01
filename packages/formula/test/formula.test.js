import test from 'node:test';
import assert from 'node:assert/strict';

import {
  indexToCol, colToIndex, parseRef, toRef, parseRange, expandRange, shiftRef,
  parse, evaluate, recalcSheet, parseCellInput, displayValue, dependencies,
  applyNumFmt, isErr, RangeValue,
} from '../src/index.js';

/* ------------------------------------------------------------------- refs */

test('column letters round-trip', () => {
  assert.equal(indexToCol(0), 'A');
  assert.equal(indexToCol(25), 'Z');
  assert.equal(indexToCol(26), 'AA');
  assert.equal(indexToCol(701), 'ZZ');
  for (const i of [0, 5, 25, 26, 51, 52, 700, 701]) {
    assert.equal(colToIndex(indexToCol(i)), i, `index ${i}`);
  }
});

test('cell references parse and print', () => {
  assert.deepEqual(parseRef('B12'), { col: 1, row: 11 });
  assert.deepEqual(parseRef('$C$3'), { col: 2, row: 2 });
  assert.equal(parseRef('B0'), null);
  assert.equal(parseRef('hello'), null);
  assert.equal(toRef(1, 11), 'B12');
});

test('ranges normalise regardless of corner order', () => {
  const a = parseRange('C3:A1');
  assert.deepEqual(a.start, { col: 0, row: 0 });
  assert.deepEqual(a.end, { col: 2, row: 2 });
  assert.deepEqual(expandRange('A1:B2'), ['A1', 'B1', 'A2', 'B2']);
});

test('shiftRef respects absolute anchors', () => {
  assert.equal(shiftRef('A1', 1, 1), 'B2');
  assert.equal(shiftRef('$A1', 1, 1), '$A2');
  assert.equal(shiftRef('A$1', 1, 1), 'B$1');
  assert.equal(shiftRef('A1', -1, 0), '#REF!');
});

/* -------------------------------------------------------------- evaluation */

function ctxOf(cells) {
  return { getValue: (ref) => (ref in cells ? cells[ref] : null) };
}
const evalIn = (formula, cells = {}) => evaluate(parse(formula), ctxOf(cells));

test('arithmetic honours precedence and associativity', () => {
  assert.equal(evalIn('=1+2*3'), 7);
  assert.equal(evalIn('=(1+2)*3'), 9);
  assert.equal(evalIn('=2^3^2'), 512); // right-associative
  assert.equal(evalIn('=-2^2'), 4); // unary binds tighter than ^ here
  assert.equal(evalIn('=10/4'), 2.5);
  assert.equal(evalIn('=50%'), 0.5);
  assert.equal(evalIn('=10+5%'), 10.05);
});

test('division by zero yields an error value', () => {
  const v = evalIn('=1/0');
  assert.ok(isErr(v));
  assert.equal(v.code, '#DIV/0!');
});

test('string concatenation and comparison', () => {
  assert.equal(evalIn('="ab"&"cd"'), 'abcd');
  assert.equal(evalIn('="a"&1'), 'a1');
  assert.equal(evalIn('=1<2'), true);
  assert.equal(evalIn('=2<>2'), false);
  assert.equal(evalIn('="a"="A"'), true, 'text comparison is case-insensitive');
  assert.equal(evalIn('=1<"a"'), true, 'numbers sort before text');
});

test('refs and ranges resolve through the context', () => {
  const cells = { A1: 1, A2: 2, A3: 3, B1: 'x' };
  assert.equal(evalIn('=A1+A2', cells), 3);
  assert.equal(evalIn('=SUM(A1:A3)', cells), 6);
  assert.equal(evalIn('=AVERAGE(A1:A3)', cells), 2);
  assert.equal(evalIn('=COUNTA(A1:B1)', cells), 2);
  assert.equal(evalIn('=MAX(A1:A3)', cells), 3);
});

test('SUM ignores text, COUNT counts only numbers', () => {
  const cells = { A1: 1, A2: 'hello', A3: 3 };
  assert.equal(evalIn('=SUM(A1:A3)', cells), 4);
  assert.equal(evalIn('=COUNT(A1:A3)', cells), 2);
  assert.equal(evalIn('=COUNTA(A1:A3)', cells), 3);
});

test('IF and IFERROR', () => {
  assert.equal(evalIn('=IF(1>0,"yes","no")'), 'yes');
  assert.equal(evalIn('=IF(1<0,"yes","no")'), 'no');
  assert.equal(evalIn('=IFERROR(1/0,"safe")'), 'safe');
  assert.equal(evalIn('=IFERROR(4/2,"safe")'), 2);
});

test('conditional aggregation', () => {
  const cells = {
    A1: 'seoul', A2: 'busan', A3: 'seoul',
    B1: 10, B2: 20, B3: 30,
  };
  assert.equal(evalIn('=SUMIF(A1:A3,"seoul",B1:B3)', cells), 40);
  assert.equal(evalIn('=COUNTIF(A1:A3,"seoul")', cells), 2);
  assert.equal(evalIn('=COUNTIF(B1:B3,">15")', cells), 2);
  assert.equal(evalIn('=AVERAGEIF(A1:A3,"seoul",B1:B3)', cells), 20);
});

test('lookup functions', () => {
  const cells = {
    A1: 'a', B1: 1,
    A2: 'b', B2: 2,
    A3: 'c', B3: 3,
  };
  assert.equal(evalIn('=VLOOKUP("b",A1:B3,2,FALSE)', cells), 2);
  assert.equal(evalIn('=MATCH("c",A1:A3,0)', cells), 3);
  assert.equal(evalIn('=INDEX(B1:B3,2)', cells), 2);
  assert.equal(evalIn('=INDEX(A1:B3,3,2)', cells), 3);
  const missing = evalIn('=VLOOKUP("zz",A1:B3,2,FALSE)', cells);
  assert.ok(isErr(missing) && missing.code === '#N/A');
});

test('text functions', () => {
  assert.equal(evalIn('=LEFT("hello",2)'), 'he');
  assert.equal(evalIn('=RIGHT("hello",2)'), 'lo');
  assert.equal(evalIn('=MID("hello",2,3)'), 'ell');
  assert.equal(evalIn('=LEN("한글")'), 2);
  assert.equal(evalIn('=UPPER("abc")'), 'ABC');
  assert.equal(evalIn('=TRIM("  a  b  ")'), 'a b');
  assert.equal(evalIn('=SUBSTITUTE("a-b-c","-","+")'), 'a+b+c');
  assert.equal(evalIn('=TEXTJOIN(", ",TRUE,"a","b")'), 'a, b');
});

test('unknown function names surface as #NAME?', () => {
  const v = evalIn('=NOPE(1)');
  assert.ok(isErr(v) && v.code === '#NAME?');
});

test('SUMPRODUCT multiplies element-wise', () => {
  const cells = { A1: 2, A2: 3, B1: 4, B2: 5 };
  assert.equal(evalIn('=SUMPRODUCT(A1:A2,B1:B2)', cells), 23);
});

/* ----------------------------------------------------------- sheet recalc */

test('recalcSheet resolves formula chains in any declaration order', () => {
  const { cells } = recalcSheet({
    cells: {
      C1: { f: '=B1*2' },
      B1: { f: '=A1+1' },
      A1: { v: 5, t: 'n' },
    },
  });
  assert.equal(cells.A1.v, 5);
  assert.equal(cells.B1.v, 6);
  assert.equal(cells.C1.v, 12);
});

test('recalcSheet reports circular references instead of hanging', () => {
  const { cells, errors } = recalcSheet({
    cells: { A1: { f: '=B1' }, B1: { f: '=A1' } },
  });
  assert.equal(cells.A1.t, 'e');
  assert.equal(errors.A1, '#CIRC!');
});

test('recalcSheet propagates errors downstream', () => {
  const { cells } = recalcSheet({
    cells: { A1: { f: '=1/0' }, B1: { f: '=A1+1' } },
  });
  assert.equal(cells.A1.v, '#DIV/0!');
  assert.equal(cells.B1.v, '#DIV/0!');
});

test('named ranges resolve in formulas', () => {
  const { cells } = recalcSheet({
    names: { sales: 'A1:A3' },
    cells: {
      A1: { v: 1 }, A2: { v: 2 }, A3: { v: 3 },
      B1: { f: '=SUM(sales)' },
    },
  });
  assert.equal(cells.B1.v, 6);
});

test('a formula referencing an empty cell treats it as zero', () => {
  const { cells } = recalcSheet({ cells: { A1: { f: '=Z99+5' } } });
  assert.equal(cells.A1.v, 5);
});

/* ------------------------------------------------------------ cell input */

test('parseCellInput classifies what the user typed', () => {
  assert.deepEqual(parseCellInput('42'), { v: 42, t: 'n' });
  assert.deepEqual(parseCellInput('1,200'), { v: 1200, t: 'n', fmt: '#,##0' });
  assert.deepEqual(parseCellInput('-3.5'), { v: -3.5, t: 'n' });
  assert.deepEqual(parseCellInput('12%'), { v: 0.12, t: 'n', fmt: '0.0%' });
  assert.deepEqual(parseCellInput('TRUE'), { v: true, t: 'b' });
  assert.deepEqual(parseCellInput('=SUM(A1:A2)'), { f: '=SUM(A1:A2)', v: null, t: 'n' });
  assert.deepEqual(parseCellInput('안녕하세요'), { v: '안녕하세요', t: 's' });
  assert.equal(parseCellInput('   '), null);

  const won = parseCellInput('₩1,500');
  assert.equal(won.v, 1500);
  assert.equal(won.fmt, '₩#,##0');

  const date = parseCellInput('2026-09-01');
  assert.equal(date.t, 'd');
  assert.equal(date.fmt, 'yyyy-mm-dd');
});

test('number formats render as the UI promises', () => {
  assert.equal(applyNumFmt(1234.5, '#,##0'), '1,235');
  assert.equal(applyNumFmt(1234.5, '#,##0.00'), '1,234.50');
  assert.equal(applyNumFmt(0.1234, '0%'), '12%');
  assert.equal(applyNumFmt(0.1234, '0.0%'), '12.3%');
  assert.equal(applyNumFmt(1500, '₩#,##0'), '₩1,500');
  assert.equal(applyNumFmt(-42, '#,##0'), '-42');
  assert.equal(applyNumFmt(null, '#,##0'), '');
});

test('displayValue shows error codes, not raw objects', () => {
  assert.equal(displayValue({ v: '#DIV/0!', t: 'e' }), '#DIV/0!');
  assert.equal(displayValue({ v: 0.30000000000000004, t: 'n' }), '0.3');
  assert.equal(displayValue({ v: true, t: 'b' }), 'TRUE');
  assert.equal(displayValue(null), '');
});

test('dependencies lists every cell a formula reads', () => {
  const deps = dependencies('=SUM(A1:A3)+B7');
  assert.deepEqual(deps.sort(), ['A1', 'A2', 'A3', 'B7']);
});

test('malformed formulas do not throw', () => {
  const { cells } = recalcSheet({ cells: { A1: { f: '=SUM(' } } });
  assert.equal(cells.A1.t, 'e');
});

/* ---------------------------------------------------- structural edits */

test('adjustRefs shifts references when a row is inserted', async () => {
  const { adjustRefs } = await import('../src/refs.js');
  assert.equal(adjustRefs('=SUM(B2:B4)', 'row', 1, 1), '=SUM(B3:B5)');
  assert.equal(adjustRefs('=A1+B2', 'row', 5, 1), '=A1+B2', 'refs above the insertion are untouched');
  assert.equal(adjustRefs('=$B$2', 'row', 0, 1), '=$B$3', 'absolute refs still move');
  assert.equal(adjustRefs('=B2', 'col', 0, 1), '=C2');
});

test('adjustRefs turns deleted references into #REF!', async () => {
  const { adjustRefs } = await import('../src/refs.js');
  assert.equal(adjustRefs('=B3', 'row', 2, -1), '=#REF!', 'the referenced row was deleted');
  assert.equal(adjustRefs('=B5', 'row', 2, -1), '=B4', 'refs below shift up');
  assert.equal(adjustRefs('=B1', 'row', 2, -1), '=B1', 'refs above are untouched');
});

test('adjustRefs leaves string literals alone', async () => {
  const { adjustRefs } = await import('../src/refs.js');
  assert.equal(adjustRefs('=IF(A1>0,"B2 is fine","B2")', 'row', 0, 1), '=IF(A2>0,"B2 is fine","B2")');
});

test('adjustRefs ignores literal values', async () => {
  const { adjustRefs } = await import('../src/refs.js');
  assert.equal(adjustRefs('hello', 'row', 0, 1), 'hello');
  assert.equal(adjustRefs('=A1', 'row', 0, 0), '=A1');
});

/* -------------------------------------------------- absolute references */

test('absolute references resolve to the same cell as relative ones', () => {
  const cells = { A1: 5, B2: 10 };
  assert.equal(evalIn('=$A$1', cells), 5);
  assert.equal(evalIn('=$A1', cells), 5);
  assert.equal(evalIn('=A$1', cells), 5);
  assert.equal(evalIn('=$B$2/$A$1', cells), 2);
  assert.equal(evalIn('=SUM($A$1:$B$2)', cells), 15);
});

test('a percentage-of-total formula anchored with $ computes correctly', () => {
  // The shape every budget sheet uses: each row divided by an absolute total.
  const { cells } = recalcSheet({
    cells: {
      A1: { v: 30 }, A2: { v: 70 },
      A3: { f: '=SUM(A1:A2)' },
      B1: { f: '=A1/$A$3' },
      B2: { f: '=A2/$A$3' },
    },
  });
  assert.equal(cells.A3.v, 100);
  assert.equal(cells.B1.v, 0.3, `B1 = ${cells.B1.v} (${cells.B1.t})`);
  assert.equal(cells.B2.v, 0.7);
  assert.notEqual(cells.B1.t, 'e', 'must not be a #DIV/0! error');
});

/* --------------------------------------------------- relative ref shifting */

test('shiftFormula translates relative references and keeps anchors', async () => {
  const { shiftFormula } = await import('../src/refs.js');
  assert.equal(shiftFormula('=B2+C2', 0, 1), '=B3+C3');
  assert.equal(shiftFormula('=SUM(B2:B4)', 1, 0), '=SUM(C2:C4)');
  assert.equal(shiftFormula('=B2*$D$1', 0, 2), '=B4*$D$1');
  assert.equal(shiftFormula('=B$2+$C3', 0, 1), '=B$2+$C4');
  assert.equal(shiftFormula('=A1', 0, 0), '=A1', 'a zero shift is a no-op');
  assert.equal(shiftFormula('42', 0, 1), '42', 'literals are untouched');
});

test('shiftFormula leaves string literals alone', async () => {
  const { shiftFormula } = await import('../src/refs.js');
  assert.equal(shiftFormula('=IF(A1>0,"A1 ok","B2 bad")', 0, 1), '=IF(A2>0,"A1 ok","B2 bad")');
});

test('shiftFormula marks references pushed off the sheet as #REF!', async () => {
  const { shiftFormula } = await import('../src/refs.js');
  assert.equal(shiftFormula('=A1', 0, -1), '=#REF!');
});
