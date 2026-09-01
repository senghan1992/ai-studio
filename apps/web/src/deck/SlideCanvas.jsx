import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { renderMarkdown, toggleWrap, continueList } from '../lib/markdown.js';
import { assetUrl, isProjectAsset } from '../api.js';
import ChartView from '../components/ChartView.jsx';

const HANDLES = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];
const SNAP = 7;
const MIN_W = 48;
const MIN_H = 28;

/**
 * The slide surface: absolutely positioned blocks, drag to move, handles to
 * resize, snapping to neighbours and canvas centre lines.
 *
 * Geometry is kept in canvas coordinates (1280x720 by default) and only divided
 * by `scale` at the point where mouse deltas come in, so what gets written to
 * layout.json is resolution-independent.
 */
export default function SlideCanvas({
  slide, scale, selectedId, editingId, folder,
  onSelect, onEdit, onChangeBlock, onChangeBlockMd, onAddBlock, onDeleteBlock,
  onContextMenu, onOpenBlock,
}) {
  const canvasRef = useRef(null);
  const [drag, setDrag] = useState(null);
  const [guides, setGuides] = useState([]);

  const canvas = slide.canvas ?? { w: 1280, h: 720, bg: '#ffffff' };
  const blocks = [...slide.blocks].sort((a, b) => (a.z ?? 0) - (b.z ?? 0));

  /** Candidate snap positions from every other block plus the canvas. */
  const snapLines = useCallback(
    (excludeId) => {
      const v = [0, canvas.w / 2, canvas.w];
      const h = [0, canvas.h / 2, canvas.h];
      for (const b of slide.blocks) {
        if (b.id === excludeId) continue;
        v.push(b.x, b.x + b.w / 2, b.x + b.w);
        h.push(b.y, b.y + b.h / 2, b.y + b.h);
      }
      return { v, h };
    },
    [slide.blocks, canvas.w, canvas.h]
  );

  const pointerStart = (event, block, mode) => {
    if (editingId === block.id) return;
    event.preventDefault();
    event.stopPropagation();
    onSelect(block.id);
    if (block.locked) return;

    event.currentTarget.setPointerCapture?.(event.pointerId);
    setDrag({
      id: block.id,
      mode,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      origin: { x: block.x, y: block.y, w: block.w, h: block.h },
      box: { x: block.x, y: block.y, w: block.w, h: block.h },
    });
  };

  const pointerMove = (event) => {
    if (!drag || event.pointerId !== drag.pointerId) return;
    const dx = (event.clientX - drag.startX) / scale;
    const dy = (event.clientY - drag.startY) / scale;

    let box = drag.mode === 'move' ? moveBox(drag.origin, dx, dy) : resizeBox(drag.origin, drag.mode, dx, dy);
    box = clamp(box, canvas);

    const shown = [];
    if (!event.altKey) {
      const lines = snapLines(drag.id);
      const snapped = applySnap(box, lines, drag.mode, shown);
      box = clamp(snapped, canvas);
    }

    setGuides(shown);
    setDrag((d) => (d ? { ...d, box } : d));
  };

  const pointerEnd = (event) => {
    if (!drag) return;
    if (event && event.pointerId !== drag.pointerId) return;
    const { id, box, origin } = drag;
    setDrag(null);
    setGuides([]);
    if (box.x !== origin.x || box.y !== origin.y || box.w !== origin.w || box.h !== origin.h) {
      onChangeBlock(id, box);
    }
  };

  // Escape cancels an in-flight drag rather than committing a half-move.
  useEffect(() => {
    if (!drag) return undefined;
    const onKey = (e) => {
      if (e.key !== 'Escape') return;
      setDrag(null);
      setGuides([]);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [drag]);

  // Arrow-key nudging, Delete, Escape — only when not typing in a block.
  useEffect(() => {
    const onKey = (e) => {
      if (editingId) return;
      const tag = document.activeElement?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
      if (!selectedId) return;
      const block = slide.blocks.find((b) => b.id === selectedId);
      if (!block) return;

      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault();
        onDeleteBlock(selectedId);
        return;
      }
      if (e.key === 'Escape') {
        onSelect(null);
        return;
      }
      if (e.key === 'Enter') {
        e.preventDefault();
        onEdit(selectedId);
        return;
      }
      const step = e.shiftKey ? 1 : 8;
      const deltas = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
      const delta = deltas[e.key];
      if (!delta) return;
      e.preventDefault();
      onChangeBlock(
        selectedId,
        clamp({ ...block, x: block.x + delta[0], y: block.y + delta[1] }, canvas),
        { merge: true }
      );
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [selectedId, editingId, slide.blocks, canvas, onChangeBlock, onDeleteBlock, onSelect, onEdit]);

  return (
    <div className="canvas-scroll" onMouseDown={(e) => e.target === e.currentTarget && onSelect(null)}>
      <div
        ref={canvasRef}
        className="canvas"
        style={{
          width: canvas.w,
          height: canvas.h,
          background: canvas.bg ?? '#fff',
          transform: `scale(${scale})`,
        }}
        onPointerMove={pointerMove}
        onPointerUp={pointerEnd}
        onPointerCancel={pointerEnd}
        onDoubleClick={(e) => {
          if (e.target !== e.currentTarget) return;
          const rect = e.currentTarget.getBoundingClientRect();
          onAddBlock({
            x: Math.round((e.clientX - rect.left) / scale) - 160,
            y: Math.round((e.clientY - rect.top) / scale) - 30,
          });
        }}
        onMouseDown={(e) => e.target === e.currentTarget && onSelect(null)}
      >
        {blocks.map((block) => {
          const live = drag?.id === block.id ? { ...block, ...drag.box } : block;
          const selected = selectedId === block.id;
          const editing = editingId === block.id;
          return (
            <Block
              key={block.id}
              block={live}
              selected={selected}
              editing={editing}
              scale={scale}
              onPointerDown={(e) => pointerStart(e, live, 'move')}
              onHandleDown={(e, mode) => pointerStart(e, live, mode)}
              onDoubleClick={() => (block.kind === 'chart' || block.kind === 'image' ? onOpenBlock?.(block) : onEdit(block.id))}
              onChangeMd={(md) => onChangeBlockMd(block.id, md)}
              onExit={() => onEdit(null)}
              folder={folder}
              onContextMenu={(e) => {
                onSelect(block.id);
                onContextMenu?.(e, block);
              }}
            />
          );
        })}

        {guides.map((g, i) => (
          <div
            key={`${g.axis}-${g.at}-${i}`}
            className={`guide guide--${g.axis === 'v' ? 'v' : 'h'}`}
            style={g.axis === 'v' ? { left: g.at } : { top: g.at }}
          />
        ))}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------- block */

function Block({
  block, selected, editing, scale, folder,
  onPointerDown, onHandleDown, onDoubleClick, onChangeMd, onExit, onContextMenu,
}) {
  const style = block.style ?? {};
  const isText = block.kind === 'text' || block.kind === 'shape';

  const boxStyle = {
    left: block.x,
    top: block.y,
    width: block.w,
    height: block.h,
    zIndex: block.z ?? 1,
    ...(block.kind === 'shape'
      ? { background: style.fill ?? '#e5e7eb', borderRadius: style.radius ?? 6 }
      : {}),
  };

  const contentStyle = {
    fontSize: style.fontSize ? `${style.fontSize}px` : undefined,
    fontWeight: style.weight,
    textAlign: style.align,
    color: style.color,
    lineHeight: style.lineHeight,
    fontStyle: style.italic ? 'italic' : undefined,
    display: 'flex',
    flexDirection: 'column',
    justifyContent: style.valign === 'middle' ? 'center' : style.valign === 'bottom' ? 'flex-end' : 'flex-start',
  };

  return (
    <div
      className={`block${selected ? ' is-selected' : ''}${editing ? ' is-editing' : ''}`}
      style={boxStyle}
      onPointerDown={onPointerDown}
      onDoubleClick={(e) => {
        e.stopPropagation();
        onDoubleClick();
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onContextMenu?.(e);
      }}
      role="group"
      aria-label={`${block.kind} 요소`}
    >
      {editing && isText ? (
        <BlockEditor value={block.md} onChange={onChangeMd} onExit={onExit} />
      ) : block.kind === 'chart' ? (
        <ChartView
          md={block.md}
          width={block.w}
          height={block.h}
          surface={isDarkish(block.style?.fill) ? block.style.fill : '#ffffff'}
        />
      ) : block.kind === 'image' ? (
        <ImageBlock block={block} folder={folder} />
      ) : (
        <div className="block__content" style={contentStyle}>
          {block.md?.trim() ? (
            <div
              className="md"
              dangerouslySetInnerHTML={{
                __html: renderMarkdown(block.md, {
                  assetResolver: (src) => (isProjectAsset(src) ? assetUrl(folder, src) : src),
                }),
              }}
            />
          ) : (
            <span className="block__placeholder">두 번 눌러 편집</span>
          )}
        </div>
      )}

      {selected && !editing && !block.locked && (
        <>
          {HANDLES.map((h) => (
            <div
              key={h}
              className={`handle handle--${h}`}
              style={{ transform: `scale(${1 / scale})` }}
              onPointerDown={(e) => onHandleDown(e, h)}
            />
          ))}
        </>
      )}
    </div>
  );
}

/** An image block, shown as the image rather than as its markdown. */
function ImageBlock({ block, folder }) {
  const match = String(block.md ?? '').match(/!\[([^\]]*)\]\(([^)\s]+)/);
  const alt = match?.[1] ?? '';
  const src = match?.[2];
  if (!src) {
    return <span className="block__placeholder">두 번 눌러 이미지를 지정</span>;
  }
  const url = isProjectAsset(src) ? assetUrl(folder, src) : src;
  return (
    <img
      className="block__image"
      src={url}
      alt={alt}
      style={{
        objectFit: block.style?.fit === 'cover' ? 'cover' : 'contain',
        borderRadius: block.style?.radius ?? 0,
      }}
      draggable={false}
    />
  );
}

/** A chart drawn on a dark shape should use the dark palette. */
function isDarkish(color) {
  const hex = String(color ?? '').replace('#', '');
  if (!/^[0-9a-fA-F]{6}$/.test(hex)) return false;
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return (r * 299 + g * 587 + b * 114) / 1000 < 128;
}

/**
 * Markdown editor shown in place of the rendered block.
 * Ctrl+B / Ctrl+I insert markdown markers, so the formatting the user applies is
 * the formatting that ends up in the .md file — no hidden rich-text layer.
 */
function BlockEditor({ value, onChange, onExit }) {
  const ref = useRef(null);
  const [text, setText] = useState(value ?? '');

  useLayoutEffect(() => {
    const el = ref.current;
    el?.focus();
    el?.setSelectionRange(el.value.length, el.value.length);
  }, []);

  const apply = (result) => {
    setText(result.value);
    onChange(result.value);
    requestAnimationFrame(() => ref.current?.setSelectionRange(result.start, result.end));
  };

  const onKeyDown = (e) => {
    const el = e.currentTarget;
    const mod = e.metaKey || e.ctrlKey;

    if (e.key === 'Escape') {
      e.preventDefault();
      onExit();
      return;
    }
    if (mod && e.key.toLowerCase() === 'b') {
      e.preventDefault();
      apply(toggleWrap(el.value, el.selectionStart, el.selectionEnd, '**'));
      return;
    }
    if (mod && e.key.toLowerCase() === 'i') {
      e.preventDefault();
      apply(toggleWrap(el.value, el.selectionStart, el.selectionEnd, '*'));
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey && !mod && el.selectionStart === el.selectionEnd) {
      const next = continueList(el.value, el.selectionStart);
      if (next) {
        e.preventDefault();
        setText(next.value);
        onChange(next.value);
        requestAnimationFrame(() => ref.current?.setSelectionRange(next.caret, next.caret));
      }
    }
  };

  return (
    <textarea
      ref={ref}
      className="block__editor"
      value={text}
      onChange={(e) => {
        setText(e.target.value);
        onChange(e.target.value);
      }}
      onKeyDown={onKeyDown}
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      spellCheck={false}
    />
  );
}

/* ---------------------------------------------------------------- geometry */

function moveBox(origin, dx, dy) {
  return { ...origin, x: Math.round(origin.x + dx), y: Math.round(origin.y + dy) };
}

function resizeBox(origin, mode, dx, dy) {
  let { x, y, w, h } = origin;
  if (mode.includes('e')) w = origin.w + dx;
  if (mode.includes('s')) h = origin.h + dy;
  if (mode.includes('w')) {
    w = origin.w - dx;
    x = origin.x + dx;
  }
  if (mode.includes('n')) {
    h = origin.h - dy;
    y = origin.y + dy;
  }
  // Keep the anchored edge fixed when the drag crosses it.
  if (w < MIN_W) {
    if (mode.includes('w')) x = origin.x + origin.w - MIN_W;
    w = MIN_W;
  }
  if (h < MIN_H) {
    if (mode.includes('n')) y = origin.y + origin.h - MIN_H;
    h = MIN_H;
  }
  return { x: Math.round(x), y: Math.round(y), w: Math.round(w), h: Math.round(h) };
}

function clamp(box, canvas) {
  const w = Math.min(Math.max(MIN_W, box.w), canvas.w);
  const h = Math.min(Math.max(MIN_H, box.h), canvas.h);
  return {
    x: Math.min(Math.max(0, box.x), canvas.w - w),
    y: Math.min(Math.max(0, box.y), canvas.h - h),
    w,
    h,
  };
}

/**
 * Nudge a box onto the nearest guide within SNAP px, recording which guides fired
 * so the canvas can draw them.
 */
function applySnap(box, lines, mode, shown) {
  const out = { ...box };
  const moving = mode === 'move';

  const vCandidates = moving
    ? [
        { value: out.x, set: (t) => (out.x = t) },
        { value: out.x + out.w / 2, set: (t) => (out.x = t - out.w / 2) },
        { value: out.x + out.w, set: (t) => (out.x = t - out.w) },
      ]
    : [
        ...(mode.includes('w') ? [{ value: out.x, set: (t) => { out.w += out.x - t; out.x = t; } }] : []),
        ...(mode.includes('e') ? [{ value: out.x + out.w, set: (t) => (out.w = t - out.x) }] : []),
      ];

  const hCandidates = moving
    ? [
        { value: out.y, set: (t) => (out.y = t) },
        { value: out.y + out.h / 2, set: (t) => (out.y = t - out.h / 2) },
        { value: out.y + out.h, set: (t) => (out.y = t - out.h) },
      ]
    : [
        ...(mode.includes('n') ? [{ value: out.y, set: (t) => { out.h += out.y - t; out.y = t; } }] : []),
        ...(mode.includes('s') ? [{ value: out.y + out.h, set: (t) => (out.h = t - out.y) }] : []),
      ];

  snapAxis(vCandidates, lines.v, 'v', shown);
  snapAxis(hCandidates, lines.h, 'h', shown);

  out.x = Math.round(out.x);
  out.y = Math.round(out.y);
  out.w = Math.round(Math.max(MIN_W, out.w));
  out.h = Math.round(Math.max(MIN_H, out.h));
  return out;
}

function snapAxis(candidates, targets, axis, shown) {
  let best = null;
  for (const candidate of candidates) {
    for (const target of targets) {
      const distance = Math.abs(candidate.value - target);
      if (distance <= SNAP && (!best || distance < best.distance)) {
        best = { distance, candidate, target };
      }
    }
  }
  if (!best) return;
  best.candidate.set(best.target);
  shown.push({ axis, at: best.target });
}
