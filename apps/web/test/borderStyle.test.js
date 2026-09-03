import test from 'node:test';
import assert from 'node:assert/strict';

import { borderCss, borderStyles } from '../src/lib/borderStyle.js';

test('a plain edge is the thin grey line the editor has always drawn', () => {
  assert.equal(borderCss(true), '1px solid #9ca3af');
  assert.equal(borderCss(false), undefined);
  assert.equal(borderCss(undefined), undefined);
});

test('an imported edge keeps its line style and colour', () => {
  // A light dotted separator from a company data template.
  assert.equal(borderCss({ style: 'dotted', color: '#e5e5e5' }), '1px dotted #e5e5e5');
  assert.equal(borderCss({ style: 'medium' }), '2px solid #9ca3af');
  assert.equal(borderCss({ style: 'double', color: '#000000' }), '3px double #000000');
  assert.equal(borderCss({ style: 'mediumDashed' }), '2px dashed #9ca3af');
});

test('an unknown style falls back to thin rather than vanishing', () => {
  assert.equal(borderCss({ style: 'wavy', color: '#ff0000' }), '1px solid #ff0000');
  assert.equal(borderCss({ style: 'none' }), undefined);
});

test('only the edges that are set become CSS properties', () => {
  assert.deepEqual(borderStyles({ t: true, r: { style: 'dotted' } }), {
    borderTop: '1px solid #9ca3af',
    borderRight: '1px dotted #9ca3af',
  });
  assert.deepEqual(borderStyles(undefined), {});
});
