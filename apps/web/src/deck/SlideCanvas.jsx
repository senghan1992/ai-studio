import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { renderMarkdown } from '../lib/markdown.js';
import { assetUrl, isProjectAsset } from '../api.js';
import MarkdownEditor from '../components/MarkdownEditor.jsx';
import ChartView from '../components/ChartView.jsx';
import ShapeView from '../components/ShapeView.jsx';
import TableView from '../components/TableView.jsx';
import { adjustHandles, adjustValueAt } from '../lib/shapeAdjust.js';
import { blockTransform, cropImageStyle } from '../lib/imageStyle.js';
import { MIN_W, MIN_H, resizeBox } from '../lib/resize.js';

const HANDLES = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];
const SNAP = 7;
/** Office snaps rotation to 15° while Shift is held. */
const ROTATE_SNAP = 15;

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
  slide, scale, selectedId, selectedIds, editingId, folder,
  onSelect, onEdit, onChangeBlock, onChangeBlockMd, onAddBlock, onDeleteBlock,
  onDuplicateDrag,
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
  // Slides whose overflowing text has already been fitted once.
  const fitted = useRef(null);

  const canvas = slide.canvas ?? { w: 1280, h: 720, bg: '#ffffff' };
  const blocks = [...slide.blocks].sort((a, b) => (a.z ?? 0) - (b.z ?? 0));
  // The ordered multi-selection; a caller that only knows one id passes just
  // the primary and every selected check still works.
  const selected = selectedIds ?? (selectedId ? [selectedId] : []);
  const single = selected.length <= 1;

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
      rotation: rotationOf(block),
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
    const isCtrl = event.ctrlKey || event.metaKey;
    const isAlt = event.altKey;
    // Resize/rotate/adjust handles keep their own modifiers (Ctrl = from the
    // centre, Shift = aspect) — copy-on-drag is a move-mode gesture only.
    if (mode !== 'move') {
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
        fellows: [],
        pendingToggle: null,
        copyArmed: false,
        duplicating: false,
        altCycle: false,
        at: null,
      });
      return;
    }
    /*
     * Office/Figma copy-on-drag: Ctrl+드래그 (Windows PowerPoint) and
     * Alt+드래그 (Mac PowerPoint, Figma) duplicate the selection.
     *
     * The click meanings have to survive alongside it: a Ctrl+click without
     * movement toggles multi-selection, an Alt+click without movement steps
     * down through the stack. So neither is applied on pointer-down — the
     * toggle/cycle fires on pointer-up only when the pointer never moved, and
     * any real movement with a modifier held latches into a duplicate drag.
     */
    if (isCtrl) {
      const already = selected.includes(block.id);
      const fellows = already
        ? selected
            .filter((id) => id !== block.id)
            .map((id) => {
              const fellow = slide.blocks.find((b) => b.id === id);
              return fellow && !fellow.locked ? { id, x: fellow.x, y: fellow.y } : null;
            })
            .filter(Boolean)
        : [];
      if (block.locked) {
        // A locked block neither moves nor copies by drag; the click still
        // toggles it in the selection.
        setDrag({
          id: block.id,
          mode,
          pointerId: event.pointerId,
          startX: event.clientX,
          startY: event.clientY,
          origin: { x: block.x, y: block.y, w: block.w, h: block.h },
          box: { x: block.x, y: block.y, w: block.w, h: block.h },
          fellows: [],
          pendingToggle: { id: block.id },
          copyArmed: false,
          duplicating: false,
          altCycle: false,
          at: null,
          lockedClick: true,
        });
        return;
      }
      event.currentTarget.setPointerCapture?.(event.pointerId);
      setDrag({
        id: block.id,
        mode,
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        origin: { x: block.x, y: block.y, w: block.w, h: block.h },
        box: { x: block.x, y: block.y, w: block.w, h: block.h },
        fellows,
        pendingToggle: { id: block.id },
        copyArmed: true,
        duplicating: false,
        altCycle: false,
        at: null,
      });
      return;
    }
    // A press on an already-selected block must not collapse the group yet:
    // Office keeps the selection until pointer-up proves it was a click, so
    // an Alt+드래그 duplicates the whole group and a plain drag moves it.
    // The single-select lands on release only when nothing moved.
    const alreadySelected = selected.includes(block.id);
    if (!alreadySelected || block.locked) onSelect(block.id);
    if (block.locked) return;

    event.currentTarget.setPointerCapture?.(event.pointerId);
    // Moving a block that is already selected carries the whole selection —
    // every other selected block's origin is captured for the live preview.
    const fellows =
      alreadySelected
        ? selected
            .filter((id) => id !== block.id)
            .map((id) => {
              const fellow = slide.blocks.find((b) => b.id === id);
              return fellow && !fellow.locked ? { id, x: fellow.x, y: fellow.y } : null;
            })
            .filter(Boolean)
        : [];
    const rect = canvasRef.current?.getBoundingClientRect();
    setDrag({
      id: block.id,
      mode,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      origin: { x: block.x, y: block.y, w: block.w, h: block.h },
      box: { x: block.x, y: block.y, w: block.w, h: block.h },
      fellows,
      pendingToggle: null,
      // Collapses the group to this block on release, but only when the
      // pointer never moved — a drag keeps (and moves or copies) the group.
      pendingSingle: alreadySelected ? { id: block.id } : null,
      /*
       * Alt+click steps down through the objects stacked at that point.
       * A plain click never moves, so the first real movement turns it into
       * a duplicate drag — which is also the existing Alt+drag = no snapping.
       * `at` is where the pointer went down, in canvas coordinates.
       */
      copyArmed: isAlt,
      duplicating: false,
      altCycle: isAlt,
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

    /*
     * Click-vs-drag arbitration for the modifier gestures.
     *
     * A Ctrl+click toggles multi-selection and an Alt+click steps down the
     * stack — but only when the pointer never really moves. Past ~4px the
     * click is over: with Ctrl or Alt held (at press or mid-drag, as in
     * Office) it latches into a duplicate drag instead. A locked Ctrl+click
     * never drags at all.
     */
    if (drag.mode === 'move' && !drag.lockedClick) {
      const dist = Math.abs(event.clientX - drag.startX) + Math.abs(event.clientY - drag.startY);
      if ((drag.altCycle || drag.pendingToggle || drag.pendingSingle) && dist <= 4 && !drag.duplicating) return;
      const copyHeld = event.altKey || event.ctrlKey || event.metaKey;
      if (!drag.duplicating && dist > 4 && (drag.copyArmed || copyHeld)) {
        // A plain drag that picks up a modifier mid-gesture copies too, as in
        // Office — but a modifier-free drag stays a move, and a handle drag
        // never copies (Ctrl there means "from the centre").
        setDrag((d) => (d && !d.duplicating ? { ...d, duplicating: true, altCycle: false, pendingToggle: null, pendingSingle: null } : d));
      } else if ((drag.altCycle || drag.pendingToggle || drag.pendingSingle) && !drag.duplicating) {
        setDrag((d) => (d ? { ...d, altCycle: false, pendingToggle: null, pendingSingle: null } : d));
      }
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

    /*
     * Shift locks the aspect ratio and Ctrl/Cmd resizes from the centre, as in
     * Office. (Alt is already taken: it turns snapping off.)
     */
    let box =
      drag.mode === 'move'
        ? moveBox(drag.origin, dx, dy)
        : resizeBox(drag.origin, drag.mode, dx, dy, {
            lockAspect: event.shiftKey,
            fromCenter: event.ctrlKey || event.metaKey,
          });
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
      if (block && rotationOf(block) !== rotation) {
        // Rotation lives on `shape` for shapes and on `style` for everything
        // else (an image has no ShapeSpec) — the renderer reads both.
        if (block.kind === 'shape') {
          onChangeBlock(id, { shape: { ...(block.shape ?? {}), rotation } });
        } else {
          onChangeBlock(id, { style: { ...(block.style ?? {}), rotation } });
        }
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

    // A click that never moved resolves the deferred modifier meaning: an
    // Alt+click steps down the stack, a Ctrl+click toggles multi-selection.
    // A real movement with a modifier instead becomes copies at the drop point.
    if (drag.mode === 'move' && !drag.lockedClick) {
      const { origin } = drag;
      const box = drag.box ?? origin;
      const moved = box.x !== origin.x || box.y !== origin.y || box.w !== origin.w || box.h !== origin.h;
      if (!moved) {
        if (drag.altCycle) {
          const { id, at } = drag;
          setDrag(null);
          setGuides([]);
          onSelect(blockBelowAt(at, id));
          return;
        }
        if (drag.pendingToggle) {
          const { id } = drag.pendingToggle;
          setDrag(null);
          setGuides([]);
          onSelect(id, { toggle: true });
          return;
        }
        if (drag.pendingSingle) {
          const { id } = drag.pendingSingle;
          setDrag(null);
          setGuides([]);
          onSelect(id);
          return;
        }
      } else if (drag.duplicating) {
        const ids = [drag.id, ...(drag.fellows ?? []).map((f) => f.id)];
        const dx = box.x - origin.x;
        const dy = box.y - origin.y;
        setDrag(null);
        setGuides([]);
        onDuplicateDrag?.(ids, dx, dy);
        return;
      }
    }
    if (drag.lockedClick) {
      // A locked block never drags: a Ctrl+press is a toggle-click candidate.
      const pending = drag.pendingToggle;
      setDrag(null);
      setGuides([]);
      if (pending) onSelect(pending.id, { toggle: true });
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
    if (drag.pendingToggle) {
      const { id } = drag.pendingToggle;
      setDrag(null);
      setGuides([]);
      onSelect(id, { toggle: true });
      return;
    }
    if (drag.pendingSingle) {
      const { id } = drag.pendingSingle;
      setDrag(null);
      setGuides([]);
      onSelect(id);
      return;
    }

    const { id, box, origin } = drag;
    setDrag(null);
    setGuides([]);
    if (box.x !== origin.x || box.y !== origin.y || box.w !== origin.w || box.h !== origin.h) {
      onChangeBlock(id, box);
      // The rest of the selection follows by the same delta.
      const dx = box.x - origin.x;
      const dy = box.y - origin.y;
      for (const fellow of drag.fellows ?? []) {
        onChangeBlock(fellow.id, { x: fellow.x + dx, y: fellow.y + dy });
      }
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
   * Grow a text box that holds more words than it has room for.
   *
   * PowerPoint never hides overflowing text: a box set to autofit grows, and a
   * box without it lets the text spill. The editor clips instead, so an imported
   * box whose type came out wider in this app's single font showed only part of
   * its words until the reader dragged the box bigger by hand. Measuring the
   * rendered content and growing the box by the difference makes the rectangle
   * wrap its text from the start. It only grows, and only once per slide visit —
   * enough to fix what was clipped without fighting a deliberate resize.
   */
  useLayoutEffect(() => {
    // Only a slide that came from an Office file: an authored slide's box is the
    // size the author chose, and silently growing it would be a change nobody
    // asked for.
    if (drag || editingId || fitted.current === slide.id || !slide.layoutPart) {
      return undefined;
    }
    const root = canvasRef.current;
    if (!root) return undefined;
    fitted.current = slide.id;
    const frame = requestAnimationFrame(() => {
      for (const block of blocks) {
        // Text and shapes both hold words. A pure image/table/chart does not.
        if ((block.kind !== 'text' && block.kind !== 'shape') || block.locked) continue;
        const content = root.querySelector(`[data-blk="${block.id}"] .block__content`);
        if (!content) continue;
        // The markdown child can be shrunk by the flex column, so its own
        // scrollHeight is measured too — otherwise a box that clips internally
        // reports no overflow.
        const inner = content.firstElementChild;
        const need = Math.max(content.scrollHeight, inner?.scrollHeight ?? 0);
        const overflow = need - content.clientHeight;
        if (overflow <= 2) continue;
        const grown = Math.min(block.h + overflow, canvas.h - block.y);
        if (grown > block.h + 1) onChangeBlock(block.id, { h: Math.round(grown) });
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [slide.id, blocks, drag, editingId, onChangeBlock, canvas.h]);

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
      if (selected.length === 0) return;
      if (activeCell && activeCell.id === selectedId) return;
      const block = slide.blocks.find((b) => b.id === selectedId);
      if (!block) return;

      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault();
        // The whole selection goes, not just the primary.
        for (const id of selected) onDeleteBlock(id);
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
      // Nudging moves the whole selection together.
      for (const id of selected) {
        const peer = slide.blocks.find((b) => b.id === id);
        if (!peer || peer.locked) continue;
        onChangeBlock(
          id,
          clamp({ ...peer, x: peer.x + delta[0], y: peer.y + delta[1] }),
          { merge: true }
        );
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [selectedId, selectedIds, editingId, activeCell, slide.blocks, canvas, onChangeBlock, onDeleteBlock, onSelect, onEdit]);

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
          // While a duplicate drag is latched the cursor carries the Office
          // copy badge, so Alt+드래그/Ctrl+드래그 reads as "놓으면 복사된다".
          cursor: drag?.duplicating ? 'copy' : pendingShape ? 'crosshair' : undefined,
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
          // A cancelled modifier-click must not silently change the selection.
          if (drag?.altCycle || drag?.pendingToggle || drag?.pendingSingle) {
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
          const isSelected = selected.includes(block.id);
          const editing = editingId === block.id;
          return (
            <Block
              key={block.id}
              block={live}
              selected={isSelected}
              single={single}
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
  if (drag && drag.mode === 'move' && block.id !== drag.id) {
    const fellow = (drag.fellows ?? []).find((f) => f.id === block.id);
    if (fellow) {
      const dx = drag.box.x - drag.origin.x;
      const dy = drag.box.y - drag.origin.y;
      return { ...block, x: fellow.x + dx, y: fellow.y + dy };
    }
    return block;
  }
  if (drag?.id !== block.id) return block;
  if (drag.mode === 'rotate') {
    if (block.kind === 'shape') {
      return { ...block, shape: { ...(block.shape ?? {}), rotation: drag.rotation } };
    }
    return { ...block, style: { ...(block.style ?? {}), rotation: drag.rotation } };
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
  block, selected, single, editing, scale, folder,
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

  return (
    <div
      className={`block${selected ? ' is-selected' : ''}${editing ? ' is-editing' : ''}`}
      style={boxStyle}
      data-blk={block.id}
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
        <MarkdownEditor
          mode="slide"
          className="block__editor block__content md md--slide"
          style={contentStyle}
          value={block.md ?? ''}
          editable
          onInput={onChangeMd}
          onExit={onExit}
          onPointerDown={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
        />
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

      {/*
        Resize/rotate/adjust handles act on one box, so they show for a single
        selection only. A multi-selection still drag-moves together.
      */}
      {selected && single && !editing && !block.locked && (
        <>
          {HANDLES.map((h) => (
            <div
              key={h}
              className={`handle handle--${h}`}
              style={{ transform: `scale(${1 / scale})` }}
              onPointerDown={(e) => onHandleDown(e, h)}
            />
          ))}

          {/*
            Office puts the rotate handle on a stem above the top edge — on
            shapes, text boxes and pictures. Tables and charts have none.
          */}
          {(block.kind === 'shape' || block.kind === 'text' || block.kind === 'image') && (
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

/* ---------------------------------------------------------------- geometry */

function moveBox(origin, dx, dy) {
  return { ...origin, x: Math.round(origin.x + dx), y: Math.round(origin.y + dy) };
}

/** Rotation lives on `shape` for shapes and on `style` for the rest. */
function rotationOf(block) {
  return block.shape?.rotation ?? block.style?.rotation ?? 0;
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
