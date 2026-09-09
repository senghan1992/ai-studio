import React, { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { FUNCTION_NAMES } from '../core/index.js';

/**
 * Excel's formula autocomplete — the dropdown that appears while a function
 * name is being typed after `=` (or inside a function's arguments): arrow
 * keys move the highlight, Enter/Tab insert `NAME(`, Esc dismisses it.
 *
 * The trigger is Excel's own: the caret must sit at the end of a run of
 * letters that starts right after `=`, `(`, `,` or an operator. `=` alone
 * offers the whole library; `SUM(` offers nothing (the caret is at an
 * argument, not a name); `SUM(A` offers the library again.
 */

/**
 * The suggestion at the caret, or `null` when the caret is not in a function
 * name position. The returned `start` is where the typed word begins in
 * `value`, which is what the picker replaces with `NAME(`.
 */
export function functionSuggestFor(value, caret) {
  const text = String(value ?? '');
  if (!text.startsWith('=')) return null;

  let start = caret;
  while (start > 0 && /[A-Za-z_]/.test(text[start - 1])) start--;
  const prev = start > 0 ? text[start - 1] : '';
  if (prev && !'=(,+-*/^&<>: '.includes(prev)) return null;

  const word = text.slice(start, caret);
  if (word) {
    const prefix = word.toUpperCase();
    const matches = FUNCTION_NAMES.filter((n) => n.startsWith(prefix));
    if (!matches.length) return null;
    return { start, matches };
  }
  // A bare `=` shows the whole library, as Excel does. An empty word after
  // `(`/`,`/an operator is an argument position — no list.
  if (prev === '=') return { start, matches: FUNCTION_NAMES };
  return null;
}

/**
 * The dropdown itself. Rendered via a portal pinned to the viewport so it
 * floats over headers and frozen panes; it scrolls closed with the sheet, the
 * way Excel's list goes away when the grid moves under it.
 */
export default function FunctionSuggestList({ anchorRef, sug, index, onHover, onPick }) {
  const listRef = useRef(null);
  const [pos, setPos] = useState(null);

  useEffect(() => {
    const el = anchorRef.current;
    if (!el) {
      setPos(null);
      return;
    }
    const measure = () => {
      const r = el.getBoundingClientRect();
      const height = Math.min(300, sug.matches.length * 26 + 6);
      // Flip above the cell when it sits too low to fit the dropdown below it.
      const top = r.bottom + 2 + height > window.innerHeight - 8 ? r.top - height - 2 : r.bottom + 2;
      setPos({ left: r.left, top });
    };
    measure();
    // The list pins to the viewport, so it re-anchors whenever the grid moves
    // under it (scroll or resize) — staying glued to the cell like Excel's.
    window.addEventListener('resize', measure);
    const scroller = el.closest('.sheet') ?? document.querySelector('.sheet');
    scroller?.addEventListener('scroll', measure, true);
    return () => {
      window.removeEventListener('resize', measure);
      scroller?.removeEventListener('scroll', measure, true);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sug]);

  // Keep the highlighted row inside the dropdown's scroll window.
  useEffect(() => {
    listRef.current?.querySelector('.fnsuggest__item.is-active')?.scrollIntoView({ block: 'nearest' });
  }, [index]);

  if (!pos) return null;

  return createPortal(
    <div className="fnsuggest" ref={listRef} style={{ left: pos.left, top: pos.top }}>
      {sug.matches.map((name, i) => (
        <button
          type="button"
          key={name}
          className={`fnsuggest__item${i === index ? ' is-active' : ''}`}
          onMouseEnter={() => onHover(i)}
          onMouseDown={(e) => e.preventDefault()}
          onClick={(e) => {
            e.preventDefault();
            onPick(name);
          }}
        >
          {name}
        </button>
      ))}
    </div>,
    document.body
  );
}