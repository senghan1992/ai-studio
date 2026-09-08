import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal, flushSync } from 'react-dom';

/**
 * True while the browser is printing.
 *
 * Print-only layouts (a deck's one-slide-per-page, a sheet's used range) are
 * mounted only for the duration: keeping them in the DOM all the time doubles
 * every block for anything that counts elements, and costs render time on
 * every edit. `flushSync` matters — the print snapshot is taken right after
 * `beforeprint`, so the mount cannot wait for the next React tick.
 */
export function usePrinting() {
  const [printing, setPrinting] = useState(false);
  useEffect(() => {
    const before = () => flushSync(() => setPrinting(true));
    const after = () => setPrinting(false);
    window.addEventListener('beforeprint', before);
    window.addEventListener('afterprint', after);
    return () => {
      window.removeEventListener('beforeprint', before);
      window.removeEventListener('afterprint', after);
    };
  }, []);
  return printing;
}

/* ------------------------------------------------------------------ ribbon */

/**
 * The ribbon.
 *
 * `contextual` tabs appear only while something that needs them is selected — a
 * shape brings up 도형 서식, a table brings up 표 디자인 and 레이아웃. Office
 * groups them to the right under a shared heading, and they vanish again when
 * the selection changes, which is why they are a separate list rather than
 * entries the caller splices into `tabs`.
 */
export function Ribbon({ tabs, active, onTab, children, contextual = [], contextLabel }) {
  return (
    <div className="ribbon">
      <div className="ribbon__tabs" role="tablist" aria-label="리본 탭">
        {tabs.map((tab) => (
          <button
            key={tab}
            role="tab"
            className="ribbon__tab"
            aria-selected={tab === active}
            onClick={() => onTab(tab)}
          >
            {tab}
          </button>
        ))}
        {contextual.length > 0 && (
          <span className="ribbon__context" aria-label={contextLabel}>
            {contextLabel && <span className="ribbon__contextlabel">{contextLabel}</span>}
            {contextual.map((tab) => (
              <button
                key={tab}
                role="tab"
                className="ribbon__tab ribbon__tab--context"
                aria-selected={tab === active}
                onClick={() => onTab(tab)}
              >
                {tab}
              </button>
            ))}
          </span>
        )}
      </div>
      <div className="ribbon__body" role="tabpanel">
        {children}
      </div>
    </div>
  );
}

export function Group({ label, children }) {
  return (
    <div className="rgroup">
      <div className="rgroup__items">{children}</div>
      <div className="rgroup__label">{label}</div>
    </div>
  );
}

export function Btn({ icon, label, onClick, disabled, pressed, title, small }) {
  return (
    <button
      type="button"
      className={small ? 'rbtn rbtn--sm' : 'rbtn'}
      onClick={onClick}
      disabled={disabled}
      title={title ?? label}
      {...(pressed === undefined ? {} : { 'aria-pressed': pressed })}
    >
      {icon && <span className="rbtn__icon" aria-hidden="true">{icon}</span>}
      <span>{label}</span>
    </button>
  );
}

export function Select({ value, onChange, options, title, width }) {
  return (
    <select
      className="rselect"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      title={title}
      aria-label={title}
      style={width ? { width } : undefined}
    >
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

export function NumInput({ value, onChange, title, min, max, step = 1 }) {
  return (
    <input
      className="rinput rinput--num"
      type="number"
      value={value}
      min={min}
      max={max}
      step={step}
      title={title}
      aria-label={title}
      onChange={(e) => {
        const n = Number(e.target.value);
        if (Number.isFinite(n)) onChange(n);
      }}
    />
  );
}

/* ------------------------------------------------------------------ popover */

/**
 * A panel that hangs off a control without being clipped by it.
 *
 * The ribbon scrolls sideways, so it clips its own children — a palette or a
 * shape gallery placed inside it gets cut off at the ribbon's edge. This renders
 * into `document.body` instead and positions itself against the control's
 * on-screen box, which is how a menu behaves everywhere else: it can overhang
 * the toolbar, the window edge flips it, and scrolling keeps it attached.
 *
 * Dismisses on a click outside, on Escape, and on the control being pressed
 * again — all three of which people try.
 */
export function Popover({ anchorRef, onClose, label, children, gap = 4 }) {
  const panel = useRef(null);
  const [box, setBox] = useState(null);

  const place = useCallback(() => {
    const anchor = anchorRef?.current;
    if (!anchor) return;
    const rect = anchor.getBoundingClientRect();
    const width = panel.current?.offsetWidth ?? 0;
    const height = panel.current?.offsetHeight ?? 0;
    const margin = 8;
    const viewport = {
      w: window.innerWidth || 1024,
      h: window.innerHeight || 768,
    };
    // Left-aligned with the control, pulled back when it would run off the
    // right edge, and never off the left.
    const left = Math.max(margin, Math.min(rect.left, viewport.w - width - margin));
    // Below by default; above when there is no room below but there is above.
    const below = rect.bottom + gap;
    const fitsBelow = below + height <= viewport.h - margin;
    const top = fitsBelow ? below : Math.max(margin, rect.top - gap - height);
    setBox({ top, left, maxHeight: Math.max(160, viewport.h - top - margin) });
  }, [anchorRef, gap]);

  useLayoutEffect(() => {
    place();
    // A scroll anywhere moves the control, so the listener is on capture.
    window.addEventListener('scroll', place, true);
    window.addEventListener('resize', place);
    return () => {
      window.removeEventListener('scroll', place, true);
      window.removeEventListener('resize', place);
    };
  }, [place]);

  useEffect(() => {
    const onAway = (e) => {
      if (panel.current?.contains(e.target) || anchorRef?.current?.contains(e.target)) return;
      onClose?.();
    };
    const onKey = (e) => e.key === 'Escape' && onClose?.();
    window.addEventListener('mousedown', onAway);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousedown', onAway);
      window.removeEventListener('keydown', onKey);
    };
  }, [anchorRef, onClose]);

  if (typeof document === 'undefined') return null;
  return createPortal(
    <div
      className="popover"
      ref={panel}
      role="dialog"
      aria-label={label}
      style={box ? { top: box.top, left: box.left, '--popover-max': `${box.maxHeight}px` } : { visibility: 'hidden' }}
    >
      {children}
    </div>,
    document.body
  );
}

/* ------------------------------------------------------------- colour picker */

/** Office's standard colour row, unchanged since it was invented. */
const STANDARD = [
  '#c00000', '#ff0000', '#ffc000', '#ffff00', '#92d050',
  '#00b050', '#00b0f0', '#0070c0', '#002060', '#7030a0',
];

/**
 * The theme row: the two backgrounds, the two text colours, then six accents.
 *
 * These are the colours this project's own `.pptx` theme writes, so a shape
 * filled from here keeps its colour when the deck is exported and reopened in
 * PowerPoint. The first accent is the document's own, which is what the theme
 * picker in the 디자인 tab sets.
 */
const themeRow = (accent) => [
  '#ffffff', '#000000', '#e7e6e6', '#44546a',
  accent || '#4472c4', '#ed7d31', '#a5a5a5', '#ffc000', '#5b9bd5', '#70ad47',
];

const channels = (hex) => {
  const v = String(hex ?? '').replace('#', '');
  const full = v.length === 3 ? v.split('').map((c) => c + c).join('') : v;
  return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16) || 0);
};

const toHex = (rgb) =>
  `#${rgb.map((n) => Math.round(Math.min(255, Math.max(0, n))).toString(16).padStart(2, '0')).join('')}`;

/** Mix toward white (`amount` > 0) or black (`amount` < 0), as Office does. */
const mix = (hex, amount) =>
  toHex(channels(hex).map((c) => (amount >= 0 ? c + (255 - c) * amount : c * (1 + amount))));

/**
 * The five variants Office lists under each theme colour.
 *
 * Light colours get darker, dark colours get lighter, and an accent goes three
 * steps lighter then two steps darker — the ramp everyone recognises from the
 * fill dropdown.
 */
function variants(hex) {
  const [r, g, b] = channels(hex);
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  if (luminance > 0.85) return [-0.05, -0.15, -0.25, -0.35, -0.5].map((a) => mix(hex, a));
  if (luminance < 0.35) return [0.5, 0.35, 0.25, 0.15, 0.05].map((a) => mix(hex, a));
  return [0.8, 0.6, 0.4, -0.25, -0.5].map((a) => mix(hex, a));
}

/**
 * A colour control shaped like Office's: a swatch showing the current colour,
 * and a palette under it.
 *
 * The palette is the one Office shows — theme colours with their tints and
 * shades, the ten standard colours, an explicit "none", and the system picker
 * for anything else. Six fixed swatches in the ribbon is the thing that makes a
 * lookalike feel like a lookalike.
 */
export function ColorPicker({ value, onChange, title, none, accent, small, disabled }) {
  const [open, setOpen] = useState(false);
  const button = useRef(null);

  const pick = (color) => {
    onChange(color);
    setOpen(false);
  };
  const theme = themeRow(accent);
  // The "다른 색…" swatch and the system picker prefill with the document's own
  // accent, falling back to the Office default the theme row also uses. The
  // native colour input only accepts six-digit hex.
  const isHex = (c) => /^#[0-9a-f]{6}$/i.test(c ?? '');
  const prefill = isHex(accent) ? accent : '#4472c4';

  return (
    <>
      <button
        type="button"
        ref={button}
        className={`colorbtn${small ? ' colorbtn--sm' : ''}`}
        title={title}
        aria-label={title}
        aria-expanded={open}
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
      >
        <span
          className={`colorbtn__swatch${value ? '' : ' is-none'}`}
          style={value ? { background: value } : undefined}
        />
        <span className="colorbtn__caret" aria-hidden="true">
          ▾
        </span>
      </button>

      {open && !disabled && (
        <Popover anchorRef={button} onClose={() => setOpen(false)} label={title}>
          <div className="palette">
          <p className="palette__label">테마 색</p>
          <div className="palette__grid">
            {theme.map((color) => (
              <button
                key={color}
                type="button"
                className="swatch"
                style={{ background: color }}
                title={color}
                aria-pressed={value?.toLowerCase() === color.toLowerCase()}
                onClick={() => pick(color)}
              />
            ))}
            {[0, 1, 2, 3, 4].map((row) =>
              theme.map((color) => {
                const shade = variants(color)[row];
                return (
                  <button
                    key={`${color}-${row}`}
                    type="button"
                    className="swatch"
                    style={{ background: shade }}
                    title={shade}
                    aria-pressed={value?.toLowerCase() === shade.toLowerCase()}
                    onClick={() => pick(shade)}
                  />
                );
              })
            )}
          </div>

          <p className="palette__label">표준 색</p>
          <div className="palette__grid">
            {STANDARD.map((color) => (
              <button
                key={color}
                type="button"
                className="swatch"
                style={{ background: color }}
                title={color}
                aria-pressed={value?.toLowerCase() === color.toLowerCase()}
                onClick={() => pick(color)}
              />
            ))}
          </div>

          <div className="palette__foot">
            {none && (
              <button type="button" className="palette__item" onClick={() => pick(null)}>
                <span className="colorbtn__swatch is-none" />
                {none}
              </button>
            )}
            <label className="palette__item">
              <span className="colorbtn__swatch" style={{ background: value || prefill }} />
              다른 색…
              <input
                type="color"
                value={isHex(value) ? value : prefill}
                onChange={(e) => onChange(e.target.value)}
                aria-label={`${title} 직접 선택`}
              />
            </label>
          </div>
          </div>
        </Popover>
      )}
    </>
  );
}

/* ------------------------------------------------------------------ dialog */

export function Dialog({ title, children, onCancel, onConfirm, confirmLabel = '확인', danger, busy }) {
  const ref = useRef(null);

  useEffect(() => {
    const first = ref.current?.querySelector('input, select, button');
    first?.focus();
    const onKey = (e) => {
      if (e.key === 'Escape') onCancel?.();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onCancel]);

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onCancel?.()}>
      <form
        ref={ref}
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onSubmit={(e) => {
          e.preventDefault();
          onConfirm?.();
        }}
      >
        <div className="dialog__head">{title}</div>
        <div className="dialog__body">{children}</div>
        <div className="dialog__foot">
          <button type="button" className="btn" onClick={onCancel}>
            취소
          </button>
          <button type="submit" className={`btn ${danger ? 'btn--danger' : 'btn--primary'}`} disabled={busy}>
            {busy ? '처리 중…' : confirmLabel}
          </button>
        </div>
      </form>
    </div>
  );
}

/* ------------------------------------------------------------------- toast */

export function Toast({ message, tone, onDone }) {
  useEffect(() => {
    if (!message) return undefined;
    const t = setTimeout(onDone, tone === 'error' ? 5000 : 2200);
    return () => clearTimeout(t);
  }, [message, tone, onDone]);

  if (!message) return null;
  return (
    <div className={`toast${tone === 'error' ? ' toast--error' : ''}`} role="status">
      {message}
    </div>
  );
}

/**
 * Toast state + a `notify` / `fail` pair, used by every editor.
 * Messages are coerced to text: callers legitimately pass an Error straight from
 * a `.catch`, and rendering an object as a React child would crash the app at the
 * exact moment it is trying to report a problem.
 */
export function useToast() {
  const [toast, setToast] = useState({ message: '', tone: 'info' });
  const show = (message, tone) => setToast({ message: asText(message), tone });
  return {
    toast,
    notify: (message) => show(message, 'info'),
    fail: (message) => show(message, 'error'),
    clear: () => setToast({ message: '', tone: 'info' }),
  };
}

function asText(value) {
  if (value === null || value === undefined) return '';
  if (typeof value === 'string') return value;
  if (value instanceof Error) return value.message || '알 수 없는 오류';
  return String(value);
}

/* ------------------------------------------------------------------ fields */

export function Field({ label, children }) {
  return (
    <label className="field">
      <span>{label}</span>
      {children}
    </label>
  );
}

/* ------------------------------------------------------------ context menu */

/**
 * Right-click menu.
 *
 * Office users reach for this constantly, and its absence is one of the first
 * things that makes a web app feel unfinished. Items are `{ label, onClick,
 * disabled, danger, shortcut }`, or the string `'-'` for a separator.
 */
export function ContextMenu({ x, y, items, onClose }) {
  const ref = useRef(null);
  // A menu opened from the keyboard carries no pointer coordinates, so fall back
  // to the top-left rather than styling the element with NaN.
  const [pos, setPos] = useState({ x: finite(x, 8), y: finite(y, 8) });

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    // Flip the menu back on screen when opened near an edge.
    const rect = el.getBoundingClientRect();
    setPos({
      x: Math.max(4, Math.min(finite(x, 8), window.innerWidth - rect.width - 8)),
      y: Math.max(4, Math.min(finite(y, 8), window.innerHeight - rect.height - 8)),
    });
  }, [x, y]);

  useEffect(() => {
    const close = () => onClose();
    const onKey = (e) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('mousedown', close);
    window.addEventListener('resize', close);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousedown', close);
      window.removeEventListener('resize', close);
      window.removeEventListener('keydown', onKey);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      className="ctxmenu"
      style={{ left: pos.x, top: pos.y }}
      role="menu"
      onMouseDown={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
    >
      {items.map((item, i) =>
        item === '-' ? (
          <div key={`sep-${i}`} className="ctxmenu__sep" role="separator" />
        ) : item.head ? (
          <div key={item.label} className="ctxmenu__head" role="presentation">{item.label}</div>
        ) : (
          <button
            key={item.label}
            type="button"
            role="menuitem"
            className={`ctxmenu__item${item.danger ? ' is-danger' : ''}`}
            disabled={item.disabled}
            onClick={() => {
              onClose();
              item.onClick?.();
            }}
          >
            <span>{item.label}</span>
            {item.shortcut && <span className="ctxmenu__key">{item.shortcut}</span>}
          </button>
        )
      )}
    </div>
  );
}

function finite(value, fallback) {
  return Number.isFinite(Number(value)) ? Number(value) : fallback;
}

/** Right-click state helper: `open(event, items)` / `close()`. */
export function useContextMenu() {
  const [menu, setMenu] = useState(null);
  return {
    menu,
    open: (event, items) => {
      event.preventDefault();
      event.stopPropagation();
      setMenu({ x: event.clientX, y: event.clientY, items });
    },
    close: () => setMenu(null),
  };
}

/* ------------------------------------------------------------ zoom control */

/** Status-bar zoom control, matching Office's slider and percentage readout. */
export function ZoomSlider({ value, onChange, min = 0.25, max = 2 }) {
  const pct = Math.round(value * 100);
  return (
    <span className="zoomctl">
      <button
        type="button"
        onClick={() => onChange(clampZoom(value - 0.1, min, max))}
        title="축소"
        aria-label="축소"
      >
        −
      </button>
      <input
        type="range"
        min={min * 100}
        max={max * 100}
        step={5}
        value={pct}
        onChange={(e) => onChange(Number(e.target.value) / 100)}
        aria-label="확대/축소"
        title={`${pct}%`}
      />
      <button
        type="button"
        onClick={() => onChange(clampZoom(value + 0.1, min, max))}
        title="확대"
        aria-label="확대"
      >
        +
      </button>
      <button type="button" className="zoomctl__pct" onClick={() => onChange(1)} title="100%로 재설정">
        {pct}%
      </button>
    </span>
  );
}

function clampZoom(v, min, max) {
  return Math.min(max, Math.max(min, Math.round(v * 100) / 100));
}

/**
 * A labelled checkbox for a ribbon group.
 *
 * Office's table style options are checkboxes, not toggle buttons — the state
 * has to be readable at a glance without hovering, because three of them decide
 * how the table looks.
 */
export function Check({ label, checked, onChange, disabled, title }) {
  return (
    <label className={`ribbon__check${disabled ? ' is-disabled' : ''}`} title={title ?? label}>
      <input
        type="checkbox"
        checked={!!checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>{label}</span>
    </label>
  );
}
