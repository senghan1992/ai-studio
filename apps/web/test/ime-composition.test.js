import test from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';

/*
 * IME composition must never be re-rendered under a live composition.
 *
 * After Enter, the fresh paragraph is `<p><br></p>` and the caret sits before
 * the `<br>`. The browser keeps that `<br>` while composing, so the live DOM
 * (`<p>글자<br></p>`) never equals the rendered markdown (`<p>글자</p>`); if
 * the editor swapped innerHTML on every input event, the browser re-anchors
 * the broken composition at the paragraph start and every keystroke
 * overwrites the first character — until a space commits it. These tests pin
 * the guard: the DOM is left to the browser during the composition and healed
 * only on compositionend.
 */

const dom = new JSDOM('<!doctype html><html><body></body></html>', { pretendToBeVisual: true });
for (const key of ['window', 'document', 'Node', 'Element', 'HTMLElement', 'Text', 'Range', 'Selection']) {
  globalThis[key] = dom.window[key];
}
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const React = (await import('react')).default;
const { act } = await import('react-dom/test-utils');
const { createRoot } = await import('react-dom/client');
const DocSurface = (await import('../src/doc/DocSurface.jsx')).default;

/** The editor's parent, mirroring DocEditor: commits update the blocks prop. */
function SurfaceWithHost({ commits, editable = true }) {
  const [blocks, setBlocks] = React.useState([{ id: 'b1', md: '', type: 'paragraph' }]);
  const [focus, setFocus] = React.useState(editable ? { id: 'b1', caret: 0 } : null);
  const noop = () => {};
  return React.createElement(DocSurface, {
    blocks,
    selectedId: editable ? 'b1' : null,
    editingId: editable ? 'b1' : null,
    focusRequest: focus,
    onFocusHandled: () => setFocus(null),
    onBlockChange: (id, md) => {
      commits.push([id, md]);
      setBlocks((bs) => bs.map((b) => (b.id === id ? { ...b, md } : b)));
    },
    onRebuild: setBlocks,
    onSplit: noop,
    onAutoSplit: noop,
    onMergeBackward: noop,
    onStepParagraph: noop,
    onIndent: noop,
    onUndo: noop,
    onRedo: noop,
    onSelect: noop,
    onCaret: noop,
    onDeselect: noop,
    onAddParagraphAtEnd: noop,
    onExit: noop,
    onContextMenu: noop,
  });
}

const mount = async () => {
  const container = document.createElement('div');
  document.body.appendChild(container);
  const root = createRoot(container);
  const commits = [];
  await act(async () => {
    root.render(React.createElement(SurfaceWithHost, { commits }));
  });
  const surface = container.querySelector('.doc-surface');
  const body = surface.querySelector('.docblk__body');
  return { root, container, surface, body, commits };
};

/** The browser's own edit: insert composed text, keeping the stale `<br>`. */
const browserType = (surface, text) => {
  const body = surface.querySelector('.docblk__body');
  body.innerHTML = `<p>${text}<br></p>`;
  const node = body.querySelector('p').firstChild;
  const range = document.createRange();
  range.setStart(node, node.data.length);
  range.collapse(true);
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(range);
  body.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
};

test('plain typing still heals the DOM and commits markdown (no composition)', async () => {
  const { surface, body, commits } = await mount();
  assert.equal(body.innerHTML, '<p><br></p>');

  // First keystroke into the empty paragraph: the browser keeps the <br>, so
  // the live DOM differs from the rendered markdown — the editor heals it and
  // restores the caret (the pre-composition behaviour stays intact). The
  // committed markdown keeps the break as a trailing newline, and the healed
  // DOM is the renderer's canonical output for it.
  await act(() => browserType(surface, 'a'));
  assert.equal(body.innerHTML, '<p>a<br>\u200B</p>\n');
  assert.deepEqual(commits, [['b1', 'a\n']]);
});

test('the composed DOM is left untouched while the composition lives', async () => {
  const { surface, body, commits } = await mount();

  await act(() => surface.dispatchEvent(new dom.window.Event('compositionstart', { bubbles: true })));

  // Three composition updates. The text is committed to markdown (the stale
  // `<br>` serializes as a trailing line break), but the DOM must keep the
  // browser's `<br>` — re-rendering would break the composition.
  for (const text of ['ㅇ', 'ㅇㅏ', 'ㅇㅏㄴ']) {
    await act(() => browserType(surface, text));
    assert.equal(body.innerHTML, `<p>${text}<br></p>`, `DOM re-rendered mid-composition at "${text}"`);
  }
  assert.deepEqual(commits, [['b1', 'ㅇ\n'], ['b1', 'ㅇㅏ\n'], ['b1', 'ㅇㅏㄴ\n']]);

  // compositionend: the browser committed its final text — now it is safe to
  // heal the DOM to the rendered form and restore the caret.
  await act(() => surface.dispatchEvent(new dom.window.Event('compositionend', { bubbles: true })));
  assert.equal(body.innerHTML, '<p>ㅇㅏㄴ<br>\u200B</p>\n');
  assert.deepEqual(commits.at(-1), ['b1', 'ㅇㅏㄴ\n']);
});