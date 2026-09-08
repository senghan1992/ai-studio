import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { renderMarkdown, toggleWrap, continueList } from '../lib/markdown.js';
import { assetUrl, isProjectAsset } from '../api.js';
import ChartView from '../components/ChartView.jsx';
import ShapeView from '../components/ShapeView.jsx';
import TableView from '../components/TableView.jsx';
import { adjustHandles, adjustValueAt } from '../lib/shapeAdjust.js';
import { blockTransform, cropImageStyle } from '../lib/imageStyle.js';

const HANDLES = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];
const SNAP = 7;
/** Office snaps rotation to 15° while Shift is held. */
const ROTATE_SNAP = 15;
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
/**
 * CSS variables for an imported deck's bullet look, per level: the glyph the
 * author used and how far the text is set in. Unset, the editor's own bullets
 * apply. Level indents are the author's `marL` deltas between levels.
 */
export function listVars(list) {
  const out = {};
  const levels = list?.levels;
  if (!Array.isArray(levels)) return out;
  let previous = 0;
  levels.slice(0, 4).forEach((level, i) => {
    if (!level) return;
    if (typeof level.glyph === 'string' && level.glyph) out[`--bullet-${i}`] = JSON.stringify(level.glyph);
    if (typeof level.marL === 'number') {
      out[`--list-indent-${i}`] = `${Math.max(level.marL - previous, 8)}px`;
      previous = level.marL;
    }
  });
  return out;
}

export default function SlideCanvas({
  slide, scale, selectedId, editingId, folder,
  onSelect, onEdit, onChangeBlock, onChangeBlockMd, onAddBlock, onDeleteBlock,
  onContextMenu, onOpenBlock,
  // A shape picked from the gallery: the next drag on the canvas draws it.
  pendingShape, onDrawShape,
  // Which table cell has the cursor, so the Layout tab can act on it.
  activeCell, onActiveCellChange,
  onChangeTable,
}) {
  const canvasRef = useRef(null);
  const [drag, setDrag] = useState(null);
  const [guides, setGuides] = useState([]);
  // The rubber band while drawing a new shape.
  const [draw, setDraw] = useState(null);

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

  /**
   * Drawing a new shape.
   *
   * Office turns the cursor into a crosshair after you pick from the gallery and
   * lets you drag out the box; a plain click drops it at a default size. Both
   * paths go through here.
   */
  const drawStart = (event) => {
    if (!pendingShape || event.button !== 0) return;
    event.preventDefault();
    const rect = event.currentTarget.getBoundingClientRect();
    const x = Math.round((event.clientX - rect.left) / scale);
    const y = Math.round((event.clientY - rect.top) / scale);
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setDraw({ pointerId: event.pointerId, x0: x, y0: y, x, y });
  };

  const drawMove = (event) => {
    if (!draw || event.pointerId !== draw.pointerId) return;
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    setDraw((d) => ({
      ...d,
      x: Math.round((event.clientX - rect.left) / scale),
      y: Math.round((event.clientY - rect.top) / scale),
    }));
  };

  const drawEnd = (event) => {
    if (!draw || (event && event.pointerId !== draw.pointerId)) return;
    const { x0, y0, x, y } = draw;
    setDraw(null);
    const w = Math.abs(x - x0);
    const h = Math.abs(y - y0);
    // Below a few pixels it was a click, not a drag: use Office's default size,
    // centred on where the pointer went down.
    const box =
      w < 8 || h < 8
        ? { x: x0 - 120, y: y0 - 70, w: 240, h: 140 }
        : { x: Math.min(x0, x), y: Math.min(y0, y), w, h };
    onDrawShape?.(clamp({ ...box, w: Math.max(MIN_W, box.w), h: Math.max(MIN_H, box.h) }));
  };

  /**
   * Rotating a shape.
   *
   * The angle is measured from the block's centre to the pointer, with the
   * handle starting straight up — so the shape follows the pointer rather than
   * jumping by the offset between them. Shift snaps to 15°, as in Office.
   */
  const rotateStart = (event, block) => {
    event.preventDefault();
    event.stopPropagation();
    onSelect(block.id);
    if (block.locked) return;
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setDrag({
      id: block.id,
      mode: 'rotate',
      pointerId: event.pointerId,
      centre: {
        x: rect.left + (block.x + block.w / 2) * scale,
        y: rect.top + (block.y + block.h / 2) * scale,
      },
      rotation: block.shape?.rotation ?? 0,
    });
  };

  /** Dragging one of a preset's adjust handles. */
  const adjustStart = (event, block, handle) => {
    event.preventDefault();
    event.stopPropagation();
    onSelect(block.id);
    if (block.locked) return;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setDrag({
      id: block.id,
      mode: 'adjust',
      pointerId: event.pointerId,
      handle,
      box: { x: block.x, y: block.y, w: block.w, h: block.h },
      value: handle.value,
    });
  };

  const pointerStart = (event, block, mode) => {
    if (editingId === block.id) return;
    event.preventDefault();
    event.stopPropagation();
    onSelect(block.id);
    if (block.locked) return;

    event.currentTarget.setPointerCapture?.(event.pointerId);
    const rect = canvasRef.current?.getBoundingClientRect();
    setDrag({
      id: block.id,
      mode,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      origin: { x: block.x, y: block.y, w: block.w, h: block.h },
      box: { x: block.x, y: block.y, w: block.w, h: block.h },
      /*
       * Alt+click steps down through the objects stacked at that point.
       * A plain click never moves, so the first real movement turns it into
       * an ordinary drag — which is also the existing Alt+drag = no snapping.
       * `at` is where the pointer went down, in canvas coordinates.
       */
      altCycle: event.altKey,
      at: rect
        ? {
            x: (event.clientX - rect.left) / scale,
            y: (event.clientY - rect.top) / scale,
          }
        : { x: block.x + block.w / 2, y: block.y + block.h / 2 },
    });
  };

  /**
   * The block directly below `id` at a canvas point, bottom-to-top order.
   * If nothing is stacked under it (or the point wraps around), the topmost
   * block at that point wins — Alt+click keeps cycling through the stack.
   */
  const blockBelowAt = (at, id) => {
    const mine = slide.blocks.find((b) => b.id === id);
    const under = [...slide.blocks]
      .filter(
        (b) =>
          b.id !== id &&
          at.x >= b.x &&
          at.x <= b.x + b.w &&
          at.y >= b.y &&
          at.y <= b.y + b.h
      )
      .sort((a, b) => (a.z ?? 0) - (b.z ?? 0));
    if (under.length === 0) return id;
    const myZ = mine?.z ?? 0;
    const below = under.filter((b) => (b.z ?? 0) < myZ);
    const pool = below.length ? below : under;
    return pool[pool.length - 1].id;
  };

  /** Every block whose box contains the pointer, bottom to top — for the
      right-click menu's "뒤에 있는 요소" list. */
  const stackedAt = (event) => {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return [];
    const x = (event.clientX - rect.left) / scale;
    const y = (event.clientY - rect.top) / scale;
    return [...slide.blocks]
      .filter((b) => x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h)
      .sort((a, b) => (a.z ?? 0) - (b.z ?? 0))
      .map((b) => b.id);
  };

  const pointerMove = (event) => {
    if (!drag || event.pointerId !== drag.pointerId) return;

    // An Alt+click stops being a click the moment it moves 4px; from there it
    // is the existing Alt+drag (move with snapping off).
    if (drag.altCycle) {
      if (Math.abs(event.clientX - drag.startX) + Math.abs(event.clientY - drag.startY) > 4) {
        setDrag((d) => (d ? { ...d, altCycle: false } : d));
      }
      return;
    }

    if (drag.mode === 'rotate') {
      const angle =
        (Math.atan2(event.clientY - drag.centre.y, event.clientX - drag.centre.x) * 180) /
          Math.PI +
        90;
      const snapped = event.shiftKey
        ? Math.round(angle / ROTATE_SNAP) * ROTATE_SNAP
        : Math.round(angle);
      // Keep it in 0..360 so the JSON never carries -725 degrees.
      setDrag((d) => (d ? { ...d, rotation: ((snapped % 360) + 360) % 360 } : d));
      return;
    }

    if (drag.mode === 'adjust') {
      const rect = canvasRef.current?.getBoundingClientRect();
      if (!rect) return;
      const point = {
        x: ((event.clientX - rect.left) / scale - drag.box.x) / Math.max(1, drag.box.w),
        y: ((event.clientY - rect.top) / scale - drag.box.y) / Math.max(1, drag.box.h),
      };
      setDrag((d) => (d ? { ...d, value: adjustValueAt(d.handle, point) } : d));
      return;
    }

    const dx = (event.clientX - drag.startX) / scale;
    const dy = (event.clientY - drag.startY) / scale;

    let box = drag.mode === 'move' ? moveBox(drag.origin, dx, dy) : resizeBox(drag.origin, drag.mode, dx, dy);
    box = clamp(box);

    const shown = [];
    if (!event.altKey) {
      const lines = snapLines(drag.id);
      const snapped = applySnap(box, lines, drag.mode, shown);
      box = clamp(snapped);
    }

    setGuides(shown);
    setDrag((d) => (d ? { ...d, box } : d));
  };

  const pointerEnd = (event) => {
    if (!drag) return;
    if (event && event.pointerId !== drag.pointerId) return;

    if (drag.mode === 'rotate') {
      const { id, rotation } = drag;
      setDrag(null);
      const block = slide.blocks.find((b) => b.id === id);
      if (block && (block.shape?.rotation ?? 0) !== rotation) {
        onChangeBlock(id, { shape: { ...(block.shape ?? {}), rotation } });
      }
      return;
    }

    if (drag.mode === 'adjust') {
      const { id, handle, value } = drag;
      setDrag(null);
      const block = slide.blocks.find((b) => b.id === id);
      if (block && (block.shape?.adjust?.[handle.name] ?? handle.value) !== value) {
        onChangeBlock(id, {
          shape: {
            ...(block.shape ?? {}),
            adjust: { ...(block.shape?.adjust ?? {}), [handle.name]: value },
          },
        });
      }
      return;
    }

    // An Alt+click (never moved) selects the object below the pointer instead
    // of dragging anything.
    if (drag.altCycle) {
      const { id, at } = drag;
      setDrag(null);
      setGuides([]);
      onSelect(blockBelowAt(at, id));
      return;
    }

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

  /*
   * Arrow-key nudging, Delete, Escape — only when not typing in a block.
   *
   * A table with the cursor in one of its cells is driven by the table's own
   * handler: without stepping aside here, Delete would clear the cell *and*
   * delete the whole table, and an arrow would move the cursor and drag the
   * table with it.
   */
  useEffect(() => {
    const onKey = (e) => {
      if (editingId) return;
      const tag = document.activeElement?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
      if (!selectedId) return;
      if (activeCell && activeCell.id === selectedId) return;
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
      /*
       * Arrow nudges by the grid; Ctrl (PowerPoint's modifier) or Shift nudges
       * by one pixel. Both are accepted because Office teaches Ctrl and the
       * habit of Shift-for-finer is widespread.
       *
       * Alt+arrow resizes instead of moving, which is the other thing the arrow
       * keys do to a selected shape.
       */
      const fine = e.ctrlKey || e.metaKey || e.shiftKey;
      const step = fine ? 1 : 8;
      const deltas = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
      const delta = deltas[e.key];
      if (!delta) return;
      e.preventDefault();
      if (e.altKey) {
        onChangeBlock(
          selectedId,
          clamp({ ...block, w: block.w + delta[0], h: block.h + delta[1] }),
          { merge: true }
        );
        return;
      }
      onChangeBlock(
        selectedId,
        clamp({ ...block, x: block.x + delta[0], y: block.y + delta[1] }),
        { merge: true }
      );
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [selectedId, editingId, activeCell, slide.blocks, canvas, onChangeBlock, onDeleteBlock, onSelect, onEdit]);

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
          cursor: pendingShape ? 'crosshair' : undefined,
        }}
        onPointerDown={drawStart}
        onPointerMove={(event) => {
          drawMove(event);
          pointerMove(event);
        }}
        onPointerUp={(event) => {
          drawEnd(event);
          pointerEnd(event);
        }}
        onPointerCancel={(event) => {
          setDraw(null);
          // A cancelled Alt+click must not silently change the selection.
          if (drag?.altCycle) {
            setDrag(null);
            setGuides([]);
          } else {
            pointerEnd(event);
          }
        }}
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
          const live = previewOf(block, drag);
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
              onRotateDown={(e) => rotateStart(e, live)}
              onAdjustDown={(e, handle) => adjustStart(e, live, handle)}
              onDoubleClick={() => (block.kind === 'chart' || block.kind === 'image' ? onOpenBlock?.(block) : onEdit(block.id))}
              onChangeMd={(md) => onChangeBlockMd(block.id, md)}
              onExit={() => onEdit(null)}
              folder={folder}
              activeCell={activeCell?.id === block.id ? activeCell : null}
              onActiveCellChange={(cell) =>
                onActiveCellChange?.(cell ? { ...cell, id: block.id } : null)
              }
              onSelectForCell={() => {
                onSelect(block.id);
                if (editingId) onEdit(null);
              }}
              onChangeTable={
                onChangeTable ? (md, table) => onChangeTable(block.id, md, table) : undefined
              }
              onContextMenu={(e) => {
                onSelect(block.id);
                onContextMenu?.(e, block, stackedAt(e));
              }}
            />
          );
        })}

        {draw && (
          <div
            className="drawband"
            style={{
              left: Math.min(draw.x0, draw.x),
              top: Math.min(draw.y0, draw.y),
              width: Math.abs(draw.x - draw.x0),
              height: Math.abs(draw.y - draw.y0),
            }}
          />
        )}

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

/**
 * The block as it should draw right now, including an in-flight drag.
 *
 * A rotate or adjust drag changes the shape spec rather than the box, so it
 * cannot be a plain spread of `drag.box` over the block.
 */
function previewOf(block, drag) {
  if (drag?.id !== block.id) return block;
  if (drag.mode === 'rotate') {
    return { ...block, shape: { ...(block.shape ?? {}), rotation: drag.rotation } };
  }
  if (drag.mode === 'adjust') {
    return {
      ...block,
      shape: {
        ...(block.shape ?? {}),
        adjust: { ...(block.shape?.adjust ?? {}), [drag.handle.name]: drag.value },
      },
    };
  }
  return { ...block, ...drag.box };
}

/* ------------------------------------------------------------------- block */

function Block({
  block, selected, editing, scale, folder,
  onPointerDown, onHandleDown, onDoubleClick, onChangeMd, onExit, onContextMenu,
  onRotateDown, onAdjustDown,
  activeCell, onActiveCellChange, onChangeTable, onSelectForCell,
}) {
  const style = block.style ?? {};
  const isText = block.kind === 'text' || block.kind === 'shape';

  const boxStyle = {
    left: block.x,
    top: block.y,
    width: block.w,
    height: block.h,
    zIndex: block.z ?? 1,
    // A shape's fill and outline are drawn by its preset geometry, not by a CSS
    // background — a diamond with a rectangular background is not a diamond.
    // Rotation/flip live on `shape` for shapes and on `style` for images.
    transform: blockTransform(block),
  };

  const contentStyle = {
    fontSize: style.fontSize ? `${style.fontSize}px` : undefined,
    fontWeight: style.weight,
    textAlign: style.align,
    color: style.color,
    lineHeight: style.lineHeight,
    fontStyle: style.italic ? 'italic' : undefined,
    // Paragraph spacing an imported slide states (px); unset keeps the editor's.
    '--space-before': style.spaceBefore != null ? `${style.spaceBefore}px` : undefined,
    '--space-after': style.spaceAfter != null ? `${style.spaceAfter}px` : undefined,
    ...listVars(style.list),
    display: 'flex',
    flexDirection: 'column',
    justifyContent: style.valign === 'middle' ? 'center' : style.valign === 'bottom' ? 'flex-end' : 'flex-start',
  };

  /* The edit box wears the block's own type style, so what you type while
     fixing a title looks like the title — the same size, weight and colour
     the rendered block has. */
  const editorStyle = {
    fontSize: style.fontSize ? `${style.fontSize}px` : undefined,
    fontWeight: style.weight,
    textAlign: style.align,
    color: style.color,
    lineHeight: style.lineHeight,
    fontStyle: style.italic ? 'italic' : undefined,
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
      {block.kind === 'shape' && (
        <ShapeView shape={block.shape} width={block.w} height={block.h} />
      )}

      {editing && isText ? (
        <BlockEditor value={block.md} onChange={onChangeMd} onExit={onExit} editorStyle={editorStyle} />
      ) : block.kind === 'table' ? (
        <TableView
          md={block.md}
          spec={block.table}
          width={block.w}
          height={block.h}
          active={activeCell}
          onActiveChange={onActiveCellChange}
          onChange={onChangeTable}
          onSelectBlock={onSelectForCell}
        />
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
              className="md md--slide"
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

          {/* Office puts the rotate handle on a stem above the top edge. */}
          {block.kind === 'shape' && (
            <div
              className="handle handle--rotate"
              title="끌어서 회전 (Shift: 15°씩)"
              style={{ transform: `scale(${1 / scale})` }}
              onPointerDown={onRotateDown}
            />
          )}

          {/* And the yellow diamonds for whatever the preset can adjust. */}
          {block.kind === 'shape' &&
            adjustHandles(block.shape).map((h) => (
              <div
                key={h.name}
                className="handle handle--adjust"
                title={`끌어서 모양 조절 (${h.name})`}
                style={{
                  left: `${h.at.x * 100}%`,
                  top: `${h.at.y * 100}%`,
                  transform: `translate(-50%, -50%) scale(${1 / scale}) rotate(45deg)`,
                }}
                onPointerDown={(e) => onAdjustDown(e, h)}
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
  const radius = block.style?.radius ?? 0;
  // A cropped image is enlarged and offset; the block wrapper's overflow clips
  // it. Otherwise it is contained (or covered) in the box as before.
  const crop = cropImageStyle(block.style?.crop);
  const imgStyle = crop
    ? { ...crop, borderRadius: radius }
    : {
        objectFit: block.style?.fit === 'cover' ? 'cover' : 'contain',
        borderRadius: radius,
      };
  return (
    <img className="block__image" src={url} alt={alt} style={imgStyle} draggable={false} />
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
function BlockEditor({ value, onChange, onExit, editorStyle }) {
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
      style={{ fontFamily: 'var(--doc-font)', whiteSpace: 'pre-wrap', overflowY: 'auto', ...editorStyle }}
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

/**
 * Placement is free: an object may extend past the canvas edge (PowerPoint
 * lets a shape bleed off the slide; the slideshow and print renderers clip to
 * the slide). Only the minimum sizes keep a block grabbable.
 */
function clamp(box) {
  return {
    x: Math.round(box.x),
    y: Math.round(box.y),
    w: Math.max(MIN_W, Math.round(box.w)),
    h: Math.max(MIN_H, Math.round(box.h)),
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
