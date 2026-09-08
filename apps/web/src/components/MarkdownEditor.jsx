import React, { useCallback, useLayoutEffect, useRef, useState } from 'react';
import { markHtml, renderMarkdown, continueList, indentLines, isListLine } from '../lib/markdown.js';
import {
  caretMdOffset, caretTextOffset, domToMd, setCaretAtTextOffset, textLength, textOffsetForMd,
  toggleInlineWrap,
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
 * `mode="doc"` is a word processor's flow (Enter splits, Backspace merges,
 * arrows cross paragraphs). `mode="slide"` is a text box (Enter makes a line
 * break or continues a list).
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
    const next = renderHtml(value);
    if (String(value ?? '') === lastMd.current && next === lastHtml.current) return;
    lastMd.current = String(value ?? '');
    lastHtml.current = next;
    setHtml(next);
  }, [value, highlight, editable, alwaysEditable, renderHtml]);

  /* Restore the caret after a structural re-render (typing `# ` + space, …). */
  useLayoutEffect(() => {
    const el = root.current;
    if (pendingCaret.current == null || !el) return;
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
      const nextHtml = renderHtml(md);
      if (md === lastMd.current && nextHtml === lastHtml.current) return;
      if (nextHtml !== lastHtml.current) {
        pendingCaret.current = caretTextOffset(el);
        lastHtml.current = nextHtml;
        setHtml(nextHtml);
      }
      lastMd.current = md;
      onInput?.(md);
    },
    [renderHtml, onInput]
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

  const inCode = () => {
    const sel = window.getSelection();
    if (!sel?.rangeCount) return false;
    const node = sel.getRangeAt(0).startContainer;
    return !!((node.nodeType === 1 ? node : node.parentElement)?.closest?.('pre'));
  };

  const onKeyDown = (e) => {
    const el = root.current;
    if (!el || !(editable || alwaysEditable)) return;
    const mod = e.metaKey || e.ctrlKey;
    const composing = e.nativeEvent?.isComposing;
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
    if (e.key === 'Backspace' && !mod && !composing && mode === 'doc') {
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
    if (e.key === 'Enter' && !composing) {
      if (mod) return; // Ctrl+Enter belongs to the window (page break)
      const md = lastMd.current;
      // Inside a code block Enter is just a newline, inserted literally so the
      // serialization keeps it (a browser <br> or <div> would not survive).
      if (inCode()) {
        e.preventDefault();
        document.execCommand('insertText', false, '\n');
        return;
      }
      if (mode === 'slide' && !e.shiftKey) {
        e.preventDefault();
        const offset = caretMdOffset(el);
        const cont = isListLine(md, offset) ? continueList(md, offset) : null;
        if (cont) commitMd(cont.value, cont.caret);
        else document.execCommand('insertLineBreak');
        return;
      }
      // doc: Shift+Enter — a plain line break inside the paragraph.
      if (e.shiftKey) return;
      e.preventDefault();
      const offset = caretMdOffset(el);
      const cont = isListLine(md, offset) ? continueList(md, offset) : null;
      if (cont) commitMd(cont.value, cont.caret);
      else onSplit?.(offset);
    }
  };

  const onPaste = (e) => {
    if (!(editable || alwaysEditable)) return;
    e.preventDefault();
    const text = e.clipboardData?.getData('text/plain') ?? '';
    document.execCommand('insertText', false, text);
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