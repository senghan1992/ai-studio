import test from 'node:test';
import assert from 'node:assert/strict';

import { resizeBox } from '../src/lib/resize.js';

/**
 * Resizing the way PowerPoint does: a plain drag is free, Shift locks the
 * aspect ratio, Ctrl/Cmd resizes from the centre. The plain-drag behaviour is
 * the existing one — these tests pin it while the modifiers are added.
 */
test('a plain east drag only changes the width', () => {
  const box = resizeBox({ x: 100, y: 100, w: 200, h: 100 }, 'e', 30, 50);
  assert.deepEqual(box, { x: 100, y: 100, w: 230, h: 100 });
});

test('a plain corner drag is free, not proportional', () => {
  const box = resizeBox({ x: 100, y: 100, w: 200, h: 100 }, 'se', 30, 50);
  assert.deepEqual(box, { x: 100, y: 100, w: 230, h: 150 });
});

test('shift on a corner locks the aspect ratio', () => {
  const box = resizeBox({ x: 100, y: 100, w: 200, h: 100 }, 'se', 30, 50, { lockAspect: true });
  assert.equal(box.w / box.h, 2);
  assert.equal(box.x, 100);
  assert.equal(box.y, 100);
});

test('shift on a side handle still keeps the ratio', () => {
  const box = resizeBox({ x: 100, y: 100, w: 200, h: 100 }, 'e', 40, 0, { lockAspect: true });
  assert.equal(box.w, 240);
  assert.equal(box.h, 120);
  // The dragged edge moved; the opposite edge stayed.
  assert.equal(box.x, 100);
});

test('centre resize keeps the middle where it was', () => {
  const box = resizeBox({ x: 100, y: 100, w: 200, h: 100 }, 'se', 30, 50, { fromCenter: true });
  assert.equal(box.x + box.w / 2, 200);
  assert.equal(box.y + box.h / 2, 150);
});

test('shift and centre compose', () => {
  const box = resizeBox({ x: 100, y: 100, w: 200, h: 100 }, 'se', 30, 50, {
    lockAspect: true,
    fromCenter: true,
  });
  assert.equal(box.w / box.h, 2);
  assert.equal(box.x + box.w / 2, 200);
  assert.equal(box.y + box.h / 2, 150);
});

test('minimum sizes still apply with modifiers', () => {
  const box = resizeBox({ x: 100, y: 100, w: 200, h: 100 }, 'se', -500, -500, {
    lockAspect: true,
    fromCenter: true,
  });
  assert.ok(box.w >= 48 && box.h >= 28, JSON.stringify(box));
});
