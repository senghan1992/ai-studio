import React, { useCallback, useLayoutEffect, useRef, useState } from 'react';
import { markHtml, renderMarkdown, continueList, indentLines, isListLine, mdLineAt, mdLineEmptyAt, mdMidHeadingOffset } from '../lib/markdown.js';
import {
  caretMdOffset, caretTextOffset, domToMd, seatForTextOffset, setCaretAtTextOffset, textLength,
  textOffsetForMd, toggleInlineWrap,
} from '../lib/richText.js';

/**
 * Rendered-only markdown editing.
 *
 * The block's markdown is drawn the same way it is on the page, and the browser
 * edits that drawn form: `#`, `*`, `-` never appear unless they are being typed
 * as shortcuts right now. Structural keys are answered in markdown terms —
 * Enter splits the block, Backspace at the start joins it, typing `# ` + space
 * turns the paragraph into a heading — and the stored text is recovered from
 * the DOM so the file on disk stays clean.
 *
 * `mode="doc"` is a word processor's flow: Enter is a line break inside the
 * paragraph, Enter on an empty line starts a new one, Backspace at the start
 * joins, arrows cross paragraphs. `mode="slide"` is a text box (Enter makes a
 * line break or continues a list).
 */
export default function MarkdownEditor({
  value = '',
  editable = false,
  /**
   * The surface never closes (a word processor's page): clicking anywhere puts
   * the caret there before any state has flipped. `editable` then only marks
   * the block the editor considers active.
   */
  alwaysEditable = false,
  mode = 'doc',
  className = '',
  style,
  highlight,
  assetResolver,
  placeholder = '빈 문단',
  focusRequest = null,
  onFocusHandled,
  onInput,
  onSplit,
  /** A heading completed below the block's first line — split it off. */
  onAutoSplit,
  onMergeBackward,
  onStepParagraph,
  onUndo,
  onRedo,
  onIndent,
  onExit,
  onContextMenu,
  onPointerDown,
  onDoubleClick,
}) {
  const root = useRef(null);
  /** The markdown the current DOM corresponds to. */
  const lastMd = useRef(String(value ?? ''));
  /** The html the current DOM was rendered from. */
  const lastHtml = useRef(null);
  /** Caret offset to restore after the next re-render. */
  const pendingCaret = useRef(null);
  /**
   * An IME composition (한글 등) is in flight: the composed DOM must not be
   * re-rendered or re-selected until it ends, or the browser re-anchors the
   * composition at the block's start and every keystroke overwrites the first
   * character. Edits are still read and committed — only the DOM waits.
   */
  const composing = useRef(false);

  const renderHtml = useCallback(
    (md) => {
      if (!String(md ?? '').trim()) {
        return editable || alwaysEditable ? '<p><br></p>' : `<p class="docblock__empty">${placeholder}</p>`;
      }
      const html = renderMarkdown(md, { assetResolver });
      return highlight ? markHtml(html, highlight) : html;
    },
    [editable, alwaysEditable, highlight, placeholder, assetResolver]
  );

  const [html, setHtml] = useState(() => renderHtml(value));
  if (lastHtml.current === null) lastHtml.current = html;

  /*
   * External changes — undo, a ribbon style, find-and-replace — swap the
   * rendered form wholesale. Edits made here never come back through this path:
   * the input handler already told `lastMd` about them. Re-rendering also
   * happens when the surface itself changes (an empty paragraph opening for
   * typing swaps its placeholder for a real caret).
   */
  useLayoutEffect(() => {
    if (composing.current) return; // 조합이 살아있는 DOM은 브라우저 손에 맡긴다
    const next = renderHtml(value);
    if (String(value ?? '') === lastMd.current && next === lastHtml.current) return;
    lastMd.current = String(value ?? '');
    lastHtml.current = next;
    setHtml(next);
  }, [value, highlight, editable, alwaysEditable, renderHtml]);

  /* Restore the caret after a structural re-render (typing `# ` + space, …). */
  useLayoutEffect(() => {
    const el = root.current;
    if (composing.current || pendingCaret.current == null || !el) return;
    const offset = pendingCaret.current;
    pendingCaret.current = null;
    if (el === document.activeElement) setCaretAtTextOffset(el, offset);
  });

  /*
   * Carets that arrive by request (split, insert, find) or by mount (a slide's
   * text box opens for typing). A click already left its caret in the DOM, so
   * it is left alone.
   */
  useLayoutEffect(() => {
    if (!editable) return;
    const el = root.current;
    if (!el) return;
    if (focusRequest) {
      setCaretAtTextOffset(el, focusRequest.caret ?? 0);
      el.focus();
      onFocusHandled?.();
      return;
    }
    if (mode === 'slide') {
      setCaretAtTextOffset(el, textLength(el));
      el.focus();
      return;
    }
    const sel = window.getSelection();
    const hasCaret = sel?.rangeCount > 0 && el.contains(sel.anchorNode);
    if (!hasCaret) {
      setCaretAtTextOffset(el, 0);
      el.focus();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editable, focusRequest, mode]);

  /**
   * The DOM is the truth after an edit: read the markdown out of it, and only
   * re-render when the structure actually changed (a completed shortcut, a
   * heading, a list).
   */
  const syncFromDom = useCallback(
    (el) => {
      const md = domToMd(el);
      // 문단 중간에서 완성된 제목(`# `)은 자기 블록으로 — 개요와 앵커의 경계를 지킨다.
      if (!composing.current && mode === 'doc' && onAutoSplit) {
        const at = mdMidHeadingOffset(md);
        if (at > 0) {
          onAutoSplit(md, at, caretMdOffset(el));
          return;
        }
      }
      const nextHtml = renderHtml(md);
      if (md === lastMd.current && nextHtml === lastHtml.current) return;
      // 조합 중에는 렌더링을 갈아끼우지 않는다 — 브라우저가 조합을 블록 첫머리에
      // 다시 고정해 첫 글자만 계속 덮어쓰게 된다. 조합이 끝나면 다시 돌아온다.
      if (!composing.current && nextHtml !== lastHtml.current) {
        pendingCaret.current = caretTextOffset(el);
        lastHtml.current = nextHtml;
        setHtml(nextHtml);
      }
      lastMd.current = md;
      onInput?.(md);
    },
    [renderHtml, onInput, mode, onAutoSplit]
  );

  /** Adopt a markdown the editor itself produced (list continuation, Tab). */
  const commitMd = useCallback(
    (md, mdCaret) => {
      const el = root.current;
      if (!el) return;
      const nextHtml = renderHtml(md);
      if (nextHtml !== lastHtml.current) {
        pendingCaret.current = textOffsetForMd(md, mdCaret ?? 0);
        lastHtml.current = nextHtml;
        setHtml(nextHtml);
      } else if (mdCaret != null) {
        setCaretAtTextOffset(el, textOffsetForMd(md, mdCaret));
      }
      lastMd.current = md;
      onInput?.(md);
    },
    [renderHtml, onInput]
  );

  const inside = (selector) => {
    const sel = window.getSelection();
    if (!sel?.rangeCount) return false;
    const node = sel.getRangeAt(0).startContainer;
    return !!((node.nodeType === 1 ? node : node.parentElement)?.closest?.(selector));
  };
  const inCode = () => inside('pre');

  const onKeyDown = (e) => {
    const el = root.current;
    if (!el || !(editable || alwaysEditable)) return;
    const mod = e.metaKey || e.ctrlKey;
    const imeComposing = e.nativeEvent?.isComposing;
    // A window-level shortcut (Ctrl+U, Ctrl+L, heading style…) can rebuild the
    // paragraph from markdown; remember the caret so it can be put back. Plain
    // typing and arrows never capture — a stale capture would yank the caret
    // back on an unrelated re-render.
    if (mod || e.altKey) pendingCaret.current = caretTextOffset(el);

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
      e.preventDefault();
      toggleInlineWrap(el, 'strong');
      syncFromDom(el);
      return;
    }
    if (mod && e.key.toLowerCase() === 'i') {
      e.preventDefault();
      toggleInlineWrap(el, 'em');
      syncFromDom(el);
      return;
    }
    if (e.key === 'Tab' && !mod && mode === 'doc') {
      if (isListLine(lastMd.current, caretMdOffset(el))) {
        e.preventDefault();
        const next = indentLines(lastMd.current, caretMdOffset(el), caretMdOffset(el), e.shiftKey);
        if (next) commitMd(next.value, next.start);
        return;
      }
      e.preventDefault();
      onIndent?.(e.shiftKey ? -1 : 1);
      return;
    }
    if (e.key === 'Backspace' && !mod && !imeComposing && mode === 'doc') {
      const sel = window.getSelection();
      const collapsed = sel && sel.rangeCount > 0 && sel.getRangeAt(0).collapsed;
      if (collapsed && caretTextOffset(el) === 0) {
        e.preventDefault();
        onMergeBackward?.(caretMdOffset(el));
      }
      return;
    }
    if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && !mod && !e.shiftKey && mode === 'doc') {
      const offset = caretTextOffset(el);
      if (e.key === 'ArrowUp' && offset === 0) {
        e.preventDefault();
        onStepParagraph?.(-1);
        return;
      }
      if (e.key === 'ArrowDown' && offset === textLength(el)) {
        e.preventDefault();
        onStepParagraph?.(1);
        return;
      }
      return;
    }
    if (e.key === 'Enter' && !imeComposing) {
      if (mod) return; // Ctrl+Enter belongs to the window (page break)
      const md = lastMd.current;
      // Inside a code block Enter is just a newline, inserted literally so the
      // serialization keeps it (a browser <br> or <div> would not survive).
      if (inCode()) {
        e.preventDefault();
        insertTextAtCaret('\n');
        return;
      }
      if (mode === 'slide' && !e.shiftKey) {
        e.preventDefault();
        const offset = caretMdOffset(el);
        const cont = isListLine(md, offset) ? continueList(md, offset) : null;
        if (cont) commitMd(cont.value, cont.caret);
        else insertLineBreak();
        return;
      }
      // doc: Shift+Enter follows the same path as Enter — a line break, or a
      // new paragraph on an empty line. Letting the browser answer would drop
      // a stray <div> that the serialization cannot keep.
      e.preventDefault();
      const offset = caretMdOffset(el);
      // 제목은 한 줄 — Enter는 항상 다음 문단으로 내려간다.
      if (inside('h1,h2,h3,h4,h5,h6')) {
        onSplit?.(offset);
        return;
      }
      if (isListLine(md, offset)) {
        // 빈 항목에서의 Enter: 마지막 항목이면 목록을 나가 새 문단, 중간이면
        // 항목을 지우고 그 사이를 잇는다.
        const { lineStart, lineEnd, before } = mdLineAt(md, offset);
        const emptyItem = /^[ \t]*(?:[-+*]|\d+[.)])[ \t]*$/.test(before) && md.slice(offset, lineEnd).trim() === '';
        if (emptyItem && md.slice(lineEnd).trim() === '') {
          onSplit?.(offset);
          return;
        }
        const cont = continueList(md, offset);
        if (cont) commitMd(cont.value, cont.caret);
        else onSplit?.(offset);
        return;
      }
      // 빈 줄에서의 Enter — 문단을 나눈다 (Enter 두 번 = 새 문단).
      if (mdLineEmptyAt(md, offset)) {
        onSplit?.(offset);
        return;
      }
      // 그 외의 Enter — 같은 문단 안의 줄바꿈.
      insertLineBreak();
    }
  };

  /**
   * Insert nodes at the caret without execCommand — its line-break behaviour
   * differs per browser and it does not exist in every DOM (jsdom). Splitting
   * the text node at the boundary keeps the drawn form intact. Manual DOM
   * changes fire no input event, so the markdown is re-read right away.
   */
  const insertNodes = (...nodes) => {
    const sel = window.getSelection();
    if (!sel?.rangeCount) return false;
    const range = sel.getRangeAt(0);
    if (!range.collapsed) {
      range.deleteContents();
      range.collapse(true);
      sel.removeAllRanges();
      sel.addRange(range);
    }
    // Seat the insertion inside the content: an element-boundary caret (the
    // end of a block, say) would drop the nodes outside every block, where
    // the serialization loses them.
    const seat = seatForTextOffset(root.current, caretTextOffset(root.current));
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
  const insertLineBreak = () => {
    if (!insertNodes(document.createElement('br'))) return;
    syncFromDom(root.current);
  };
  const insertTextAtCaret = (text) => {
    if (!insertNodes(document.createTextNode(text))) return;
    syncFromDom(root.current);
  };

  const onPaste = (e) => {
    if (!(editable || alwaysEditable)) return;
    e.preventDefault();
    const text = e.clipboardData?.getData('text/plain') ?? '';
    // Pasted newlines are line breaks in the same paragraph, matching Enter.
    const nodes = [];
    String(text).split('\n').forEach((part, i) => {
      if (i > 0) nodes.push(document.createElement('br'));
      if (part) nodes.push(document.createTextNode(part));
    });
    if (nodes.length && insertNodes(...nodes)) syncFromDom(root.current);
  };

  return (
    <div
      ref={root}
      className={className}
      contentEditable={alwaysEditable || editable}
      role={alwaysEditable || editable ? 'textbox' : undefined}
      aria-multiline="true"
      suppressContentEditableWarning
      spellCheck={false}
      style={style}
      onInput={() => syncFromDom(root.current)}
      onKeyDown={onKeyDown}
      onCompositionStart={() => {
        composing.current = true;
      }}
      onCompositionEnd={() => {
        composing.current = false;
        // The composition committed its text; adopt it and heal the DOM now
        // that re-rendering is safe again.
        syncFromDom(root.current);
      }}
      onPaste={onPaste}
      onBlur={() => editable && onExit?.()}
      onContextMenu={(e) => {
        if (!editable) e.preventDefault();
        onContextMenu?.(e);
      }}
      onPointerDown={onPointerDown}
      onDoubleClick={onDoubleClick}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}