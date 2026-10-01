import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';

import { shapePath, canDraw, isOpenShape, presetRotation, markerPath } from '../src/lib/shapeSvg.js';

/**
 * Every preset in the Rust gallery must have a path here.
 *
 * The gallery is what the shape picker offers, so a preset with no path is a
 * button that inserts a rectangle. Reading the list from the Rust source rather
 * than duplicating it means adding a shape in one place fails here until it is
 * drawable.
 */
test('the web renderer draws every preset the gallery offers', () => {
  const source = readFileSync(
    new URL('../../../crates/ai-format/src/shape.rs', import.meta.url),
    'utf8'
  );
  const gallery = source.slice(
    source.indexOf('pub static PRESETS'),
    source.indexOf('static BY_NAME')
  );
  const names = [...gallery.matchAll(/"([a-zA-Z0-9]+)"\s*=>/g)].map((m) => m[1]);

  assert.ok(names.length > 100, `only found ${names.length} presets`);
  const missing = names.filter((name) => !canDraw(name));
  assert.deepEqual(missing, [], `presets with no path: ${missing.join(', ')}`);
});

test('an unknown preset has no path, so the caller can fall back', () => {
  assert.equal(shapePath('swooshArrow', null), null);
  assert.equal(canDraw('swooshArrow'), false);
});

test('paths stay inside the 0..100 box', () => {
  const source = readFileSync(new URL('../src/lib/shapeSvg.js', import.meta.url), 'utf8');
  const names = [...source.matchAll(/case '([a-zA-Z0-9]+)':/g)].map((m) => m[1]);

  for (const name of new Set(names)) {
    const path = shapePath(name, null);
    assert.ok(path, `${name} has no path`);
    // Every coordinate in the path must be within a small margin of the box.
    for (const [value] of path.matchAll(/-?\d+(?:\.\d+)?/g)) {
      const n = Number(value);
      assert.ok(n >= -60 && n <= 160, `${name}: coordinate ${n} is far outside the box`);
    }
  }
});

test('adjust handles change the geometry', () => {
  // A rounded rectangle with a bigger radius must produce a different path.
  const small = shapePath('roundRect', { adjust: { adj: 5000 } });
  const large = shapePath('roundRect', { adjust: { adj: 50000 } });
  assert.notEqual(small, large);

  // And an out-of-range handle is clamped rather than producing nonsense.
  const absurd = shapePath('roundRect', { adjust: { adj: 900000 } });
  for (const [value] of absurd.matchAll(/-?\d+(?:\.\d+)?/g)) {
    assert.ok(Number(value) <= 100, `radius escaped the box: ${absurd}`);
  }
});

test('lines and braces are open paths, closed shapes are not', () => {
  assert.ok(isOpenShape('line'));
  assert.ok(isOpenShape('bracePair'));
  assert.ok(!isOpenShape('rect'));
  assert.ok(!isOpenShape('star5'));
});

test('the arrow family reuses one path with a rotation', () => {
  assert.equal(shapePath('leftArrow', null), shapePath('rightArrow', null));
  assert.equal(presetRotation('leftArrow'), 180);
  assert.equal(presetRotation('upArrow'), -90);
  assert.equal(presetRotation('rightArrow'), 0);
  assert.equal(presetRotation('rect'), 0);
});

test('every stored line-end marker has a path', () => {
  for (const type of ['triangle', 'stealth', 'diamond', 'oval', 'arrow']) {
    const path = markerPath(type);
    assert.ok(path, `${type} has no marker path`);
    for (const [value] of path.matchAll(/-?\d+(?:\.\d+)?/g)) {
      const n = Number(value);
      assert.ok(n >= 0 && n <= 10, `${type}: coordinate ${n} escapes the 0..10 marker box`);
    }
  }
});

test('a wide roundRect rounds against the short side', () => {
  // Office measures the corner against `ss` (the short side): the same shape
  // in a wide box and a square box has the same corner in pixels. A uniform
  // radius over-rounds every wide rectangle.
  const path = shapePath('roundRect', { adjust: { adj: 16667 } }, { w: 600, h: 120 });
  // 16667 is a 20px radius here: 3.333 across, 16.667 down.
  assert.ok(path.includes('A3.333,16.667'), path);
});

test('a square roundRect draws as before', () => {
  const path = shapePath('roundRect', { adjust: { adj: 16667 } });
  assert.ok(path.includes('A16.667,16.667'), path);
});

test('a wide snip cuts the same pixels on both axes', () => {
  const path = shapePath('snip1Rect', { adjust: { adj: 16667 } }, { w: 600, h: 120 });
  assert.ok(path.includes('96.667') && path.includes('16.667'), path);
});

test('a plain line end has no marker path', () => {
  assert.equal(markerPath('none'), null);
  assert.equal(markerPath('solid'), null);
  assert.equal(markerPath(undefined), null);
});

test('a star has two vertices per point', () => {
  const path = shapePath('star5', null);
  const vertices = path.split('L').length;
  assert.equal(vertices, 10, path);
});
