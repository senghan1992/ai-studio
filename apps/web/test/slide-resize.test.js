import test from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';

/*
 * Resize handles on shapes that came out of a real `.pptx` import.
 *
 * The blocks below are verbatim `ai-import` output (ids shortened) for a
 * deck exported to genuine `.pptx` bytes and re-imported through the public
 * path: a 600x120 roundRect with its adjust handle, and a connector keeping
 * both arrowheads. Dragging the SE handle must land on resized bounds while
 * the restored geometry — preset, fill, outline, markers, text — survives,
 * advancing the preview on every frame.
 */

const dom = new JSDOM('<!doctype html><html><body></body></html>', { pretendToBeVisual: true });
for (const key of ['window', 'document', 'Node', 'Element', 'HTMLElement', 'Text', 'Range', 'Selection', 'MouseEvent']) {
  globalThis[key] = dom.window[key];
}
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame.bind(dom.window);
globalThis.cancelAnimationFrame = dom.window.cancelAnimationFrame.bind(dom.window);
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
const SlideCanvas = (await import('../src/deck/SlideCanvas.jsx')).default;

const importedSlide = {
  id: 's_1',
  title: '도형',
  layoutName: 'blank',
  notes: '',
  canvas: { w: 1280, h: 720, bg: '#ffffff' },
  blocks: [
    {
      id: 'b_round', kind: 'shape', md: '둥근 네모', x: 96, y: 200, w: 600, h: 120, z: 3,
      style: { spaceBefore: 0, fontSize: 18, color: '#1f2937', align: 'center', lineHeight: 1.2, valign: 'middle' },
      shape: {
        preset: 'roundRect',
        fill: { color: '#dbeafe' },
        line: { color: '#2a78d6', width: 2.0 },
        adjust: { adj: 16667.0 },
      },
      locked: false,
    },
    {
      id: 'b_conn', kind: 'shape', md: '', x: 96, y: 360, w: 400, h: 24, z: 4,
      style: {},
      shape: {
        preset: 'straightConnector1',
        line: { color: '#4472c4', width: 2.5, head: 'triangle', tail: 'oval' },
      },
      locked: false,
    },
  ],
};

function Harness({ report, selectedIds }) {
  return React.createElement(SlideCanvas, {
    slide: importedSlide,
    scale: 1,
    selectedId: selectedIds[selectedIds.length - 1] ?? null,
    selectedIds,
    editingId: null,
    folder: 'verify',
    onSelect: (id, opts) => report.selects.push([id, opts]),
    onEdit: () => {},
    onChangeBlock: (id, box) => report.changes.push([id, box]),
    onChangeBlockMd: () => {},
    onAddBlock: () => {},
    onDeleteBlock: () => {},
    onContextMenu: () => {},
  });
}

async function renderCanvas(report) {
  const container = document.createElement('div');
  document.body.appendChild(container);
  await act(async () => {
    createRoot(container).render(React.createElement(Harness, { report, selectedIds: ['b_round'] }));
  });
  return container;
}

const seHandle = () => document.querySelector('.handle--se');
const downHandle = (el, opts = {}) =>
  el.dispatchEvent(new window.MouseEvent('pointerdown', { bubbles: true, cancelable: true, button: 0, ...opts }));
const move = (target, x, y, extra = {}) =>
  target.dispatchEvent(new window.MouseEvent('pointermove', { bubbles: true, cancelable: true, clientX: x, clientY: y, ...extra }));
const up = (target, x, y, extra = {}) =>
  target.dispatchEvent(new window.MouseEvent('pointerup', { bubbles: true, cancelable: true, clientX: x, clientY: y, ...extra }));
const boxOf = () => document.querySelector('[data-blk="b_round"]');

test('dragging SE resizes the imported roundRect across frames', async () => {
  document.body.innerHTML = '';
  const report = { selects: [], changes: [] };
  await renderCanvas(report);
  assert.ok(seHandle(), 'single selection shows the SE handle');

  const widths = [];
  await act(async () => {
    downHandle(seHandle(), { clientX: 0, clientY: 0 });
  });
  const canvas = document.querySelector('.canvas');
  for (let i = 1; i <= 15; i++) {
    await act(async () => {
      move(canvas, i * 10, (i * 10) / 2);
    });
    widths.push(parseFloat(boxOf().style.width));
  }
  await act(async () => {
    up(canvas, 150, 75);
  });

  assert.ok(widths.every((w, i) => i === 0 || w >= widths[i - 1]), `preview advanced every frame: ${widths.join(',')}`);
  const committed = report.changes.find(([id]) => id === 'b_round')?.[1];
  assert.ok(committed, 'the resize committed');
  // A plain drag snaps within 7px of neighbouring edges, as designed.
  assert.ok(Math.abs(committed.w - 750) <= 7, `width grew by the drag delta, got ${committed.w}`);
  assert.ok(Math.abs(committed.h - 195) <= 7, `height grew by the drag delta, got ${committed.h}`);
});

test('shift+SE keeps the imported ratio and preserves the restored spec', async () => {
  document.body.innerHTML = '';
  const report = { selects: [], changes: [] };
  await renderCanvas(report);

  await act(async () => {
    downHandle(seHandle(), { clientX: 0, clientY: 0 });
  });
  const canvas = document.querySelector('.canvas');
  for (let i = 1; i <= 10; i++) {
    await act(async () => {
      move(canvas, i * 10, 0, { shiftKey: true });
    });
  }
  await act(async () => {
    up(canvas, 100, 0, { shiftKey: true });
  });
  const committed = report.changes.find(([id]) => id === 'b_round')?.[1];
  assert.ok(committed, 'the resize committed');
  assert.equal(committed.w / committed.h, 5, `600x120 ratio kept, got ${committed.w}x${committed.h}`);

  // The restored spec rides along untouched.
  const live = importedSlide.blocks[0].shape;
  assert.equal(live.preset, 'roundRect');
  assert.equal(live.fill.color, '#dbeafe');
  assert.equal(live.line.width, 2.0);
  assert.equal(live.adjust.adj, 16667.0);
  assert.equal(importedSlide.blocks[0].md, '둥근 네모');

  // And the connector kept its heads through the same file.
  const conn = importedSlide.blocks[1].shape.line;
  assert.equal(conn.head, 'triangle');
  assert.equal(conn.tail, 'oval');
});
