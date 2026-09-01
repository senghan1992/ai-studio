import React, { useEffect, useRef, useState } from 'react';

/* ------------------------------------------------------------------ ribbon */

export function Ribbon({ tabs, active, onTab, children }) {
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

export function ColorRow({ colors, value, onChange, title }) {
  return (
    <div className="rrow" role="group" aria-label={title}>
      {colors.map((c) => (
        <button
          key={c}
          type="button"
          className="swatch"
          style={{ background: c }}
          aria-pressed={value?.toLowerCase() === c.toLowerCase()}
          title={c}
          onClick={() => onChange(c)}
        />
      ))}
    </div>
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
