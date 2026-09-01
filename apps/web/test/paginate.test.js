import test from 'node:test';
import assert from 'node:assert/strict';

import { paginate, isPageBreak, PAGE_BREAK, contentHeightOf, contentWidthOf } from '../src/doc/paginate.js';

const blocksOf = (...heights) =>
  heights.map((h, i) => ({ id: `b${i}`, md: `block ${i}`, _h: h }));
const heightsOf = (blocks) => Object.fromEntries(blocks.map((b) => [b.id, b._h]));

test('blocks that fit stay on one page', () => {
  const blocks = blocksOf(100, 100, 100);
  const pages = paginate(blocks, heightsOf(blocks), 400);
  assert.equal(pages.length, 1);
  assert.equal(pages[0].length, 3);
});

test('a block that would overflow moves to the next page whole', () => {
  const blocks = blocksOf(300, 300, 300);
  const pages = paginate(blocks, heightsOf(blocks), 700);
  assert.deepEqual(pages.map((p) => p.map((b) => b.id)), [['b0', 'b1'], ['b2']]);
});

test('an explicit page break starts a new page', () => {
  const blocks = [
    { id: 'a', md: 'first', _h: 50 },
    { id: 'br', md: PAGE_BREAK },
    { id: 'b', md: 'second', _h: 50 },
  ];
  const pages = paginate(blocks, heightsOf(blocks.filter((b) => b._h)), 800);
  assert.deepEqual(pages.map((p) => p.map((b) => b.id)), [['a'], ['b']]);
});

test('the break marker itself is never rendered as content', () => {
  const blocks = [{ id: 'br', md: PAGE_BREAK }, { id: 'a', md: 'x', _h: 10 }];
  const pages = paginate(blocks, { a: 10 }, 800);
  assert.ok(!pages.flat().some((b) => isPageBreak(b)));
});

test('a block taller than the page gets its own page rather than disappearing', () => {
  const blocks = blocksOf(50, 2000, 50);
  const pages = paginate(blocks, heightsOf(blocks), 600);
  const ids = pages.map((p) => p.map((b) => b.id));
  assert.deepEqual(ids, [['b0'], ['b1'], ['b2']]);
  assert.ok(pages.every((p) => p.length > 0));
});

test('unmeasured blocks are assumed to fit so the first pass renders something', () => {
  const blocks = blocksOf(undefined, undefined);
  const pages = paginate(blocks, {}, 600);
  assert.equal(pages.length, 1);
  assert.equal(pages[0].length, 2);
});

test('every block lands on exactly one page', () => {
  const blocks = blocksOf(...Array.from({ length: 40 }, (_, i) => 30 + (i % 7) * 40));
  const pages = paginate(blocks, heightsOf(blocks), 900);
  const placed = pages.flat().map((b) => b.id);
  assert.equal(placed.length, blocks.length);
  assert.equal(new Set(placed).size, blocks.length);
  assert.deepEqual(placed, blocks.map((b) => b.id), 'and in the original order');
});

test('no page exceeds the limit unless a single block does', () => {
  const blocks = blocksOf(...Array.from({ length: 30 }, (_, i) => 80 + (i % 5) * 60));
  const heights = heightsOf(blocks);
  const limit = 800;
  for (const page of paginate(blocks, heights, limit)) {
    const total = page.reduce((sum, b) => sum + heights[b.id], 0);
    assert.ok(page.length === 1 || total <= limit, `page total ${total} exceeds ${limit}`);
  }
});

test('an empty document still yields one page', () => {
  assert.deepEqual(paginate([], {}, 800), [[]]);
});

test('consecutive breaks do not produce a run of blank pages', () => {
  const blocks = [
    { id: 'a', md: 'x', _h: 10 },
    { id: 'b1', md: PAGE_BREAK },
    { id: 'b2', md: PAGE_BREAK },
    { id: 'b', md: 'y', _h: 10 },
  ];
  const pages = paginate(blocks, { a: 10, b: 10 }, 800);
  assert.deepEqual(pages.map((p) => p.map((x) => x.id)), [['a'], ['b']]);
});

test('page geometry accounts for margins', () => {
  assert.equal(contentHeightOf({ w: 794, h: 1123 }, { top: 72, bottom: 72 }), 979);
  assert.equal(contentWidthOf({ w: 794, h: 1123 }, { left: 72, right: 72 }), 650);
  assert.equal(contentHeightOf({ w: 794, h: 1123 }, undefined), 979, 'defaults apply');
});
