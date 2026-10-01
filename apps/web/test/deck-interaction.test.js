import test from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';

/*
 * The deck canvas, driven the way a person drives it: press Duplicate on the
 * toolbar, Ctrl+click two shapes, drag them together.
 *
 * `ime-composition.test.js` set the pattern — jsdom globals first, dynamic
 * imports after, `act` around every dispatch. These tests pin the behaviours
 * a unit test cannot see: the button wires the copy through with fresh ids
 * (no `id: undefined` to fail `save_project`, no throw to blank the screen),
 * Ctrl+click toggles membership with both blocks highlighted, and a group
 * drag advances every selected block frame by frame.
 */

const dom = new JSDOM('<!doctype html><html><body></body></html>', { pretendToBeVisual: true });
for (const key of ['window', 'document', 'Node', 'Element', 'HTMLElement', 'Text', 'Range', 'Selection', 'MouseEvent']) {
  globalThis[key] = dom.window[key];
}
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
if (!globalThis.window.matchMedia) {
  globalThis.window.matchMedia = () => ({ matches: false, addListener() {}, removeListener() {} });
}
if (!globalThis.Element.prototype.scrollIntoView) {
  globalThis.Element.prototype.scrollIntoView = () => {};
}

const { loadCore } = await import('./core.mjs');
await loadCore();
const React = (await import('react')).default;
const { act } = await import('react-dom/test-utils');
const { createRoot } = await import('react-dom/client');
const DeckEditor = (await import('../src/deck/DeckEditor.jsx')).default;
const { duplicateSlideAt } = await import('../src/deck/blockOps.js');
const { localSlideId, newBlockId } = await import('../src/core/index.js');

const shape = (id, x, md) => ({
  id, kind: 'shape', md, x, y: 100, w: 200, h: 100, z: 1,
  style: { fontSize: 20 },
  shape: { preset: 'rect', fill: { color: '#dbeafe', opacity: 100 } },
  table: null, locked: false,
});

function Host({ report }) {
  const [slides, setSlides] = React.useState([
    {
      id: 's_1', title: '표지', layout_name: 'blank', notes: '',
      canvas: { w: 1280, h: 720, bg: '#ffffff' },
      blocks: [shape('b_1', 100, '첫째'), shape('b_2', 400, '둘째')],
    },
  ]);
  report.slides = slides;
  const ctl = {
    project: { type: 'deck', folder: 'verify', manifest: { title: '검증' } },
    items: slides,
    setItems: (updater) => setSlides((prev) => (typeof updater === 'function' ? updater(prev) : updater)),
    setTitle: () => {},
    save: async () => null,
    saving: false, dirty: false, savedAt: null,
    undo: () => {}, redo: () => {}, canUndo: false, canRedo: false,
  };
  return React.createElement(DeckEditor, { ctl, onHome: () => {}, notify: () => {}, onNewProject: () => {} });
}

const report = { slides: null };
const container = document.createElement('div');
document.body.appendChild(container);

await act(async () => {
  createRoot(container).render(React.createElement(Host, { report }));
});

const down = (el, opts) =>
  el.dispatchEvent(new window.MouseEvent('pointerdown', { bubbles: true, cancelable: true, button: 0, ...opts }));

test('pressing Duplicate adds a slide with fresh ids and keeps the canvas alive', async () => {
  const button = document.querySelector('button[title="복제"]');
  assert.ok(button, 'the toolbar duplicate button renders');
  await act(async () => {
    button.click();
  });
  assert.equal(report.slides.length, 2);
  assert.equal(document.querySelectorAll('.sorter-item').length, 2);
  const copy = report.slides[1];
  assert.ok(copy.id && copy.id !== 's_1', `fresh slide id, got ${copy.id}`);
  assert.equal(copy.title, '표지 사본');
  // The save payload keeps every id: a dropped `id` was the original failure.
  const payload = JSON.parse(JSON.stringify({ slides: report.slides }));
  assert.equal(payload.slides[1].id, copy.id);
  assert.ok(payload.slides[1].blocks.every((b) => typeof b.id === 'string' && b.id.length > 0));
});

test('duplicating on a stale wasm still produces a saveable copy', () => {
  // The stale-binary configuration: `newSlideId` missing from wasm, so the
  // editor falls back to `localSlideId`, while `newBlockId` predates it.
  const next = duplicateSlideAt(report.slides.slice(0, 1), 0, { slideId: localSlideId, blockId: newBlockId });
  const payload = JSON.parse(JSON.stringify(next[1]));
  assert.match(payload.id, /^s_[a-z0-9]{5}$/, `fallback slide id, got ${payload.id}`);
  assert.ok(payload.blocks.every((b) => typeof b.id === 'string' && b.id.length > 0));
});

test('ctrl+click multi-selects and a drag carries the group', async () => {
  const ids = [...document.querySelectorAll('.canvas [data-blk]')].map((el) => el.getAttribute('data-blk'));
  assert.ok(ids.length >= 2, 'two shapes on the current slide');
  const blk = (id) => document.querySelector(`[data-blk="${id}"]`);
  await act(async () => {
    down(blk(ids[0]), { ctrlKey: true, clientX: 150, clientY: 150 });
  });
  await act(async () => {
    down(blk(ids[1]), { ctrlKey: true, clientX: 450, clientY: 150 });
  });
  assert.equal(document.querySelectorAll('.block.is-selected').length, 2);
  assert.equal(document.querySelectorAll('.handle--se').length, 0, 'no resize handles while multi-selected');

  const canvas = document.querySelector('.canvas');
  await act(async () => {
    down(blk(ids[0]), { clientX: 150, clientY: 150 });
  });
  for (let i = 1; i <= 10; i++) {
    await act(async () => {
      canvas.dispatchEvent(
        new window.MouseEvent('pointermove', { bubbles: true, cancelable: true, clientX: 150 + i * 5, clientY: 150 })
      );
    });
  }
  await act(async () => {
    canvas.dispatchEvent(new window.MouseEvent('pointerup', { bubbles: true, cancelable: true, clientX: 200, clientY: 150 }));
  });
  const cur = report.slides[report.slides.length - 1];
  const a = cur.blocks.find((b) => b.id === ids[0]);
  const b = cur.blocks.find((b) => b.id === ids[1]);
  assert.ok(a.x > 100 && b.x > 400, `both moved: ${a.x}, ${b.x}`);
});
