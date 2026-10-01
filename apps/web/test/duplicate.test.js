import test from 'node:test';
import assert from 'node:assert/strict';

import { loadCore } from './core.mjs';

import { duplicateSlideAt } from '../src/deck/blockOps.js';
import { duplicateSheetAt } from '../src/grid/gridOps.js';

await loadCore();
const { newSlideId, newSheetId } = await import('../src/core/index.js');

/**
 * Duplicating a slide or a sheet must produce an object the saver accepts:
 * every persisted object carries a string `id`, and `JSON.stringify` drops
 * `undefined` — so a copy left with `id: undefined` fails `save_project`
 * with "missing field `id`".
 */
test('a duplicated slide carries fresh ids and survives a save round trip', () => {
  const slides = [
    {
      id: 's_abc',
      title: '원본',
      canvas: { w: 1280, h: 720, bg: '#fff' },
      blocks: [{ id: 'b_1', kind: 'text', md: '본문', x: 0, y: 0, w: 10, h: 10 }],
    },
  ];
  let n = 0;
  const ids = { slideId: () => `s_new${n++}`, blockId: () => `b_new${n++}` };

  const next = duplicateSlideAt(slides, 0, ids);
  assert.equal(next.length, 2);
  assert.equal(next[0].id, 's_abc', 'the source keeps its id');
  const copy = next[1];
  assert.ok(typeof copy.id === 'string' && copy.id.length > 0, 'the copy has an id');
  assert.notEqual(copy.id, 's_abc');
  assert.equal(copy.title, '원본 사본');
  assert.notEqual(copy.blocks[0].id, 'b_1', 'blocks are re-identified too');
  assert.equal(copy.blocks[0].md, '본문');

  // What the saver sends: no `undefined` may remain, or the field is dropped
  // and the Rust side reports a missing field.
  const json = JSON.parse(JSON.stringify(copy));
  assert.equal(json.id, copy.id);
  assert.equal(json.blocks[0].id, copy.blocks[0].id);
});

test('a duplicated sheet carries a fresh id', () => {
  const sheets = [{ id: 'sh_1', name: '시트1', cells: {} }];
  const next = duplicateSheetAt(sheets, 0, () => 'sh_2');
  assert.equal(next.length, 2);
  assert.equal(next[0].id, 'sh_1');
  assert.equal(next[1].id, 'sh_2');
  assert.equal(next[1].name, '시트1 사본');
  const json = JSON.parse(JSON.stringify(next[1]));
  assert.equal(json.id, 'sh_2');
});

test('the core hands out slide and sheet ids', () => {
  const a = newSlideId();
  const b = newSlideId();
  assert.ok(a.startsWith('s_'), a);
  assert.notEqual(a, b);
  const c = newSheetId();
  assert.ok(c.startsWith('sh_'), c);
});
