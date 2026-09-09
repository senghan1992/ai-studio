import React, { useLayoutEffect, useMemo, useRef, useState } from 'react';
import { markHtml, renderMarkdown, continueList, indentLines, isListLine, mdLineAt, mdLineEmptyAt, mdMidHeadingOffset } from '../lib/markdown.js';
import {
  caretMdOffset, caretTextOffset, domToMd, seatForTextOffset, setCaretAtTextOffset, textLength,
  textOffsetForMd, toggleInlineWrap,
} from '../lib/richText.js';

/**
 * The whole page as one edit surface.
 *
 * The document is stored as markdown blocks, but a writer should never meet
 * the seams between them: DocSurface renders every block of a page inside a
 * single contentEditable, so clicking anywhere puts the caret exactly there,
 * arrows walk across paragraphs, the whole page can be selected and copied,
 * and Enter inside a paragraph is a line break — the same continuous flow a
 * word processor's page has. The block structure still exists underneath:
 * each block lives in its own wrapper (`data-blk`), and every edit is
 * serialized back into that block's markdown, so the file on disk keeps its
 * per-block anchors and formatting.
 */

export default function DocSurface({
  blocks = [],
  assetResolver,
  highlight,
  blockStyle = () => ({}),
  isTextless = () => false,
  textlessView = () => null,
  selectedId = null,
  editingId = null,
  focusRequest = null,
  onFocusHandled,
  onBlockChange,
  onRebuild,
  onSplit,
  onAutoSplit,
  onMergeBackward,
  onStepParagraph,
  onIndent,
  onUndo,
  onRedo,
  onSelect,
  onCaret,
  onDeselect,
  onAddParagraphAtEnd,
  onExit,
  onContextMenu,
}) {
  const root = useRef(null);
  const elByBlock = useRef(new Map());
  /** The markdown each wrapper's DOM currently corresponds to. */
  const lastMd = useRef(new Map());
  const lastCaretId = useRef(null);
  /** Caret (markdown offset in a block) to restore after a re-render. */
  const pending = useRef(null);
  /**
   * An IME composition (한글 등) is in flight.
   *
   * While it lives, the composed DOM must not be re-rendered by hand: the
   * browser tracks the composition against the very nodes we would remove,
   * and it silently re-anchors a broken composition at the paragraph's start
   * — every keystroke then overwrites the first character until a space
   * commits it. Text edits are still read and committed; only the DOM and the
   * selection are left alone, and both are healed the moment the composition
   * ends.
   */
  const composing = useRef(false);
  const [, setTick] = useState(0);

  const blocksById = useMemo(() => new Map(blocks.map((b) => [b.id, b])), [blocks]);

  const renderHtmlFor = (md) => {
    const text = String(md ?? '');
    if (!text.trim()) return '<p><br></p>';
    const html = renderMarkdown(text, { assetResolver });
    return highlight ? markHtml(html, highlight) : html;
  };

  const wrapperAt = (node) => {
    if (!node) return null;
    const el = node.nodeType === 1 ? node : node.parentElement;
    return el ? el.closest('[data-blk]') : null;
  };

  const caretWrapper = () => {
    const sel = window.getSelection();
    if (!sel || !sel.rangeCount) return null;
    return wrapperAt(sel.getRangeAt(0).startContainer);
  };

  /** The caret's markdown offset inside a wrapper, or null if it is not there. */
  const captureCaret = (wrapper) => {
    const sel = window.getSelection();
    if (!sel || !sel.rangeCount) return null;
    const node = sel.getRangeAt(0).startContainer;
    if (!wrapper.contains(node)) return null;
    return caretMdOffset(wrapper);
  };

  const reportCaret = () => {
    const wrapper = caretWrapper();
    const id = wrapper ? wrapper.dataset.blk : null;
    if (id === lastCaretId.current) return;
    lastCaretId.current = id;
    onCaret?.(id ? blocksById.get(id) ?? null : null);
  };

  /**
   * Read a wrapper's DOM back into markdown. Structural changes (typing `# ` +
   * space, a completed `---`) need the wrapper re-rendered; the caret is
   * captured first and restored after.
   */
  const syncWrapper = (wrapper, { autoSplit = true } = {}) => {
    const blockId = wrapper.dataset.blk;
    const body = wrapper.querySelector('.docblk__body') ?? wrapper;
    const md = domToMd(body);
    if (!composing.current && autoSplit && onAutoSplit) {
      const at = mdMidHeadingOffset(md);
      if (at > 0) {
        onAutoSplit(blockId, md, at, caretMdOffset(body));
        return;
      }
    }
    const changed = md !== lastMd.current.get(blockId);
    if (changed) {
      lastMd.current.set(blockId, md);
      onBlockChange?.(blockId, md);
    }
    const nextHtml = renderHtmlFor(md);
    if (!composing.current && nextHtml !== body.innerHTML) {
      const at = captureCaret(body);
      if (at !== null) {
        pending.current = { blockId, mdOffset: at };
        setTick((t) => t + 1);
      }
    }
  };

  /** Adopt markdown the editor itself produced (list continuation, Tab). */
  const commitWrapper = (wrapper, md, mdCaret) => {
    const blockId = wrapper.dataset.blk;
    const body = wrapper.querySelector('.docblk__body') ?? wrapper;
    const changed = md !== lastMd.current.get(blockId);
    if (changed) {
      lastMd.current.set(blockId, md);
      onBlockChange?.(blockId, md);
    }
    const nextHtml = renderHtmlFor(md);
    if (nextHtml !== body.innerHTML) {
      pending.current = { blockId, mdOffset: mdCaret ?? captureCaret(body) ?? 0 };
      setTick((t) => t + 1);
    } else if (mdCaret != null) {
      setCaretAtTextOffset(body, textOffsetForMd(md, mdCaret));
    }
  };

  /**
   * The browser deleted or merged wrappers (select-all + type, Delete across
   * a boundary): rebuild the page's blocks from whatever wrappers survive.
   * Text typed directly into the surface root becomes a block of its own.
   */
  const rebuildFromDom = () => {
    const rootEl = root.current;
    if (!rootEl || !blocks.length) return;
    const parts = [];
    let textBuf = '';
    const flush = () => {
      if (textBuf.trim()) parts.push({ md: textBuf });
      textBuf = '';
    };
    for (const child of [...rootEl.childNodes]) {
      if (child.nodeType === 3) {
        textBuf += child.data;
        continue;
      }
      if (child.nodeType !== 1) continue;
      const block = blocksById.get(child.dataset?.blk);
      if (block) {
        flush();
        const body = child.querySelector('.docblk__body') ?? child;
        parts.push({ id: block.id, md: isTextless(block) ? block.md : domToMd(body) });
      } else {
        const md = domToMd(child); // a wrapper the browser created itself
        if (md.trim()) {
          flush();
          parts.push({ md });
        }
      }
    }
    flush();
    if (parts.length === blocks.length && parts.every((p, i) => p.id === blocks[i].id)) return;
    const next = parts.map((p, i) => {
      const block = p.id ? blocksById.get(p.id) : null;
      return block
        ? { ...block, md: p.md }
        : { id: `new-${i}`, md: p.md, type: null, override: null };
    });
    for (const b of next) lastMd.current.set(b.id, b.md);
    onRebuild?.(next);
    const last = rootEl.querySelector('[data-blk]:last-of-type') ?? rootEl.lastElementChild;
    if (last) {
      const fixed = last.hasAttribute('data-blk') ? last : wrapperAt(last);
      if (fixed) {
        setCaretAtTextOffset(fixed, textLength(fixed));
        rootEl.focus({ preventScroll: true });
      }
    }
  };

  const onSurfaceInput = (e) => {
    // The browser's event target is the wrapper that changed; the caret may
    // be elsewhere while jsdom tests drive the DOM by hand.
    let wrapper = caretWrapper() ?? wrapperAt(e?.target);
    if (wrapper && root.current.contains(wrapper)) {
      syncWrapper(wrapper);
      reportCaret();
      return;
    }
    rebuildFromDom();
  };

  /* ------------------------------------------------------------ inserting */

  const insertNodes = (wrapper, ...nodes) => {
    const sel = window.getSelection();
    if (!sel || !sel.rangeCount) return false;
    const range = sel.getRangeAt(0);
    if (!range.collapsed) {
      range.deleteContents();
      range.collapse(true);
      sel.removeAllRanges();
      sel.addRange(range);
    }
    // Seat the insertion inside the block's content: an element-boundary caret
    // (the end of a wrapper, say) would drop the nodes outside every block,
    // where the serialization loses them.
    const seat = seatForTextOffset(wrapper, caretTextOffset(wrapper));
    const at = document.createRange();
    if (seat?.node) at.setStart(seat.node, seat.offset);
    else if (seat?.br) {
      if (seat.after) at.setStartAfter(seat.br);
      else at.setStartBefore(seat.br);
    } else {
      at.setStart(range.startContainer, range.startOffset);
    }
    at.collapse(true);
    let last = null;
    for (const node of nodes) {
      at.insertNode(node);
      at.setStartAfter(node);
      at.collapse(true);
      last = node;
    }
    if (last) {
      at.setStartAfter(last);
      at.collapse(true);
      sel.removeAllRanges();
      sel.addRange(at);
    }
    return true;
  };

  const insertLineBreak = (wrapper) => {
    const body = wrapper.querySelector('.docblk__body') ?? wrapper;
    if (insertNodes(body, document.createElement('br'))) syncWrapper(wrapper);
  };

  const insertTextAtCaret = (wrapper, text) => {
    const body = wrapper.querySelector('.docblk__body') ?? wrapper;
    if (insertNodes(body, document.createTextNode(text))) syncWrapper(wrapper);
  };

  /* ----------------------------------------------------------------- keys */

  const onKeyDown = (e) => {
    const rootEl = root.current;
    if (!rootEl) return;
    const mod = e.metaKey || e.ctrlKey;
    const imeComposing = e.nativeEvent?.isComposing;
    const sel = window.getSelection();
    const node = sel && sel.rangeCount ? sel.getRangeAt(0).startContainer : null;
    const wrapper = wrapperAt(node) ?? wrapperAt(e.target);
    const block = wrapper ? blocksById.get(wrapper.dataset.blk) ?? null : null;
    const body = wrapper ? wrapper.querySelector('.docblk__body') ?? wrapper : null;

    if (mod || e.altKey) {
      if (wrapper) pending.current = { blockId: wrapper.dataset.blk, mdOffset: caretMdOffset(body) };
      else pending.current = null;
    }

    if (e.key === 'Escape') {
      e.preventDefault();
      onExit?.();
      return;
    }
    if (mod && !e.shiftKey && e.key.toLowerCase() === 'z') {
      e.preventDefault();
      onUndo?.();
      return;
    }
    if (mod && !e.altKey && (e.key.toLowerCase() === 'y' || (e.shiftKey && e.key.toLowerCase() === 'z'))) {
      e.preventDefault();
      onRedo?.();
      return;
    }
    if (mod && e.key.toLowerCase() === 'b') {
      if (!wrapper) return;
      e.preventDefault();
      toggleInlineWrap(wrapper, 'strong');
      syncWrapper(wrapper);
      return;
    }
    if (mod && e.key.toLowerCase() === 'i') {
      if (!wrapper) return;
      e.preventDefault();
      toggleInlineWrap(wrapper, 'em');
      syncWrapper(wrapper);
      return;
    }
    if (e.key === 'Tab' && !mod) {
      if (wrapper && body && block && isListLine(block.md, caretMdOffset(body))) {
        e.preventDefault();
        const next = indentLines(block.md, caretMdOffset(body), caretMdOffset(body), e.shiftKey);
        if (next) commitWrapper(wrapper, next.value, next.start);
        return;
      }
      e.preventDefault();
      onIndent?.(e.shiftKey ? -1 : 1);
      return;
    }
    if (e.key === 'Backspace' && !mod && !imeComposing) {
      if (wrapper && body && sel && sel.rangeCount > 0 && sel.getRangeAt(0).collapsed && caretTextOffset(body) === 0) {
        e.preventDefault();
        onMergeBackward?.(wrapper.dataset.blk, caretMdOffset(body));
      }
      return;
    }
    if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && !mod && !e.shiftKey && !imeComposing) {
      if (!wrapper || !body) return;
      const offset = caretTextOffset(body);
      if (e.key === 'ArrowUp' && offset === 0) {
        e.preventDefault();
        onStepParagraph?.(wrapper.dataset.blk, -1);
        return;
      }
      if (e.key === 'ArrowDown' && offset === textLength(body)) {
        e.preventDefault();
        onStepParagraph?.(wrapper.dataset.blk, 1);
        return;
      }
      return;
    }
    if (e.key === 'Enter' && !imeComposing) {
      if (mod) return; // Ctrl+Enter belongs to the window (page break)
      if (!wrapper || !block) return;
      const md = block.md;
      const atNode = node.nodeType === 1 ? node : node.parentElement;
      // Inside a code block Enter is a literal newline — a <br> would not
      // survive the trip back to markdown.
      if (atNode?.closest?.('pre')) {
        e.preventDefault();
        insertTextAtCaret(wrapper, '\n');
        return;
      }
      e.preventDefault();
      // Word-style soft return: Shift+Enter breaks the line inside the same
      // paragraph instead of starting a new one.
      if (e.shiftKey) {
        insertLineBreak(wrapper);
        return;
      }
      const offset = caretMdOffset(body);
      // A heading is one line — Enter always opens the next paragraph.
      if (atNode?.closest?.('h1,h2,h3,h4,h5,h6')) {
        onSplit?.(wrapper.dataset.blk, offset);
        return;
      }
      if (isListLine(md, offset)) {
        // Enter on an empty item: the last one closes the list into a new
        // paragraph, one in the middle deletes itself and joins its sides.
        const { lineStart, lineEnd, before } = mdLineAt(md, offset);
        const emptyItem = /^[ \t]*(?:[-+*]|\d+[.)])[ \t]*$/.test(before) && md.slice(offset, lineEnd).trim() === '';
        if (emptyItem && md.slice(lineEnd).trim() === '') {
          onSplit?.(wrapper.dataset.blk, offset);
          return;
        }
        const cont = continueList(md, offset);
        if (cont) commitWrapper(wrapper, cont.value, cont.caret);
        else onSplit?.(wrapper.dataset.blk, offset);
        return;
      }
      // Enter on an empty line starts a new paragraph (two Enters in a row).
      if (mdLineEmptyAt(md, offset)) {
        onSplit?.(wrapper.dataset.blk, offset);
        return;
      }
      // Any other Enter starts a new paragraph, like Word.
      onSplit?.(wrapper.dataset.blk, offset);
    }
  };

  const onKeyUp = (e) => {
    if (!['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Backspace', 'Delete'].includes(e.key)) return;
    reportCaret();
    // A deleted wrapper (the browser emptied it) is caught here when the input
    // event did not fire; the wrapper-count check makes it a no-op otherwise.
    if ((e.key === 'Backspace' || e.key === 'Delete') && !caretWrapper()) rebuildFromDom();
  };

  const onPaste = (e) => {
    const wrapper = caretWrapper();
    e.preventDefault();
    if (!wrapper) return;
    const text = e.clipboardData?.getData('text/plain') ?? '';
    // Pasted newlines are line breaks in the same paragraph, matching Enter.
    const nodes = [];
    String(text).split('\n').forEach((part, i) => {
      if (i > 0) nodes.push(document.createElement('br'));
      if (part) nodes.push(document.createTextNode(part));
    });
    if (!nodes.length) return;
    const body = wrapper.querySelector('.docblk__body') ?? wrapper;
    if (insertNodes(body, ...nodes)) syncWrapper(wrapper);
  };

  /* -------------------------------------------------------------- pointers */

  const onMouseDown = (e) => {
    if (e.target === root.current) {
      onDeselect?.();
      return;
    }
    const wrapper = wrapperAt(e.target);
    if (wrapper) onSelect?.(blocksById.get(wrapper.dataset.blk));
  };

  const onDoubleClick = (e) => {
    if (e.target === root.current) onAddParagraphAtEnd?.();
  };

  const onBlur = (e) => {
    if (!root.current?.contains(e.relatedTarget)) {
      lastCaretId.current = null;
      onCaret?.(null);
    }
  };

  const handleContextMenu = (e) => {
    const wrapper = wrapperAt(e.target);
    const block = wrapper ? blocksById.get(wrapper.dataset.blk) ?? null : null;
    if (block) {
      e.preventDefault();
      onSelect?.(block);
    } else if (e.target === root.current) {
      e.preventDefault();
    } else {
      return;
    }
    onContextMenu?.(e, block ?? null);
  };

  /* ----------------------------------------------------------------- sync */

  /*
   * Own the wrappers' text, not React: after every commit, redraw each wrapper
   * whose drawn html no longer matches its markdown (a structural change, an
   * undo, a find highlight), keeping a captured caret in place.
   */
  useLayoutEffect(() => {
    for (const block of blocks) {
      const el = elByBlock.current.get(block.id);
      if (!el) continue;
      lastMd.current.set(block.id, block.md ?? '');
      // Textless blocks (tables, charts) are React-owned widgets; redrawing
      // them here would destroy React's children and explode on the next
      // commit ("node to be removed is not a child"). The text bodies below
      // are this effect's to own.
      if (isTextless(block)) continue;
      const html = renderHtmlFor(block.md ?? '');
      if (html !== el.innerHTML) {
        // 조합이 살아있는 동안에는 브라우저가 만든 DOM을 손대지 않는다.
        // innerHTML 교체는 조합 앵커를 부수고, 다음 키 입력이 문단 첫머리부터
        // 덮어쓰는 결과를 낳는다 — 조합이 끝나면 (onCompositionEnd) 정리된다.
        if (composing.current) continue;
        const at = captureCaret(el);
        if (at !== null) pending.current = { blockId: block.id, mdOffset: at };
        el.innerHTML = html;
      }
    }
    if (!composing.current && pending.current) {
      const { blockId, mdOffset } = pending.current;
      pending.current = null;
      const el = elByBlock.current.get(blockId);
      const block = blocksById.get(blockId);
      if (el && block) setCaretAtTextOffset(el, textOffsetForMd(block.md ?? '', mdOffset ?? 0));
    }
  });

  /* A requested caret (split, insert, find): place it and consume the request. */
  useLayoutEffect(() => {
    if (!focusRequest) return;
    const el = elByBlock.current.get(focusRequest.id);
    if (!el) return;
    setCaretAtTextOffset(el, focusRequest.caret ?? 0);
    const rootEl = root.current;
    if (rootEl && document.activeElement !== rootEl) rootEl.focus({ preventScroll: true });
    onFocusHandled?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusRequest]);

  /* ----------------------------------------------------------------- render */

  const wrapperClass = (block) =>
    `docblk${selectedId === block.id ? ' is-selected' : ''}${editingId === block.id ? ' is-editing' : ''}${
      block.override?.style?.fontSize ? ' md--sized' : ''
    }`;

  return (
    <div
      ref={root}
      className="doc-surface md md--doc"
      contentEditable
      role="textbox"
      aria-multiline="true"
      suppressContentEditableWarning
      spellCheck={false}
      onInput={onSurfaceInput}
      onKeyDown={onKeyDown}
      onCompositionStart={() => {
        composing.current = true;
      }}
      onCompositionEnd={() => {
        composing.current = false;
        // The composition committed its text; adopt it and heal the DOM now
        // that re-rendering is safe again.
        const wrapper = caretWrapper();
        if (wrapper && root.current.contains(wrapper)) syncWrapper(wrapper);
      }}
      onKeyUp={onKeyUp}
      onPaste={onPaste}
      onMouseDown={onMouseDown}
      onDoubleClick={onDoubleClick}
      onBlur={onBlur}
      onContextMenu={handleContextMenu}
    >
      {blocks.map((block) =>
        isTextless(block) ? (
          <div
            key={block.id}
            id={`block-${block.id}`}
            data-blk={block.id}
            contentEditable={false}
            className={`docblk docblk--static${selectedId === block.id ? ' is-selected' : ''}`}
            style={blockStyle(block)}
            ref={(el) => {
              if (el) elByBlock.current.set(block.id, el);
              else elByBlock.current.delete(block.id);
            }}
          >
            {textlessView(block)}
            {selectedId === block.id && block.override && <span className="docblock__badge">meta.json</span>}
          </div>
        ) : (
          <div
            key={block.id}
            id={`block-${block.id}`}
            data-blk={block.id}
            className={wrapperClass(block)}
            style={blockStyle(block)}
          >
            {/*
              The drawn markdown lives in its own node so React can keep the
              badge beside it; the layout effect owns the body's text alone.
            */}
            <div
              className="docblk__body"
              ref={(el) => {
                if (el) elByBlock.current.set(block.id, el);
                else elByBlock.current.delete(block.id);
              }}
            />
            {selectedId === block.id && block.override && <span className="docblock__badge">meta.json</span>}
          </div>
        )
      )}
    </div>
  );
}