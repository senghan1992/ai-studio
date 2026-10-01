import test from 'node:test';
import assert from 'node:assert/strict';

import { selectSingle, toggleInSelection } from '../src/deck/selection.js';

/**
 * Ctrl+click (Cmd+click) toggles a block in the selection without touching
 * the rest; a plain click replaces it. The canvas keeps the ordered list and
 * treats the last entry as the primary (inspector target, nudge anchor).
 */
test('a plain click selects exactly one block', () => {
  assert.deepEqual(selectSingle(['a', 'b'], 'c'), ['c']);
  assert.deepEqual(selectSingle(['a'], null), []);
});

test('ctrl+click adds a block to the selection', () => {
  assert.deepEqual(toggleInSelection(['a'], 'b'), ['a', 'b']);
});

test('ctrl+click on a selected block removes it again', () => {
  assert.deepEqual(toggleInSelection(['a', 'b'], 'a'), ['b']);
});

test('toggling the last selected block clears the selection', () => {
  assert.deepEqual(toggleInSelection(['a'], 'a'), []);
});
