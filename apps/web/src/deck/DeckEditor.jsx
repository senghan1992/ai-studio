import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  makeSlide, newBlockId, SLIDE_LAYOUTS, positionPhrase, autoLayout, clampBox,
  serializeChartBlock, parseChartBlock, makeShape, makeTable, tableEdit, tableInfo,
  readingOrder,
} from '../core/index.js';

import Shell from '../components/Shell.jsx';
import FileInspector from '../components/FileInspector.jsx';
import FileMenu from '../components/FileMenu.jsx';
import ChartDialog from '../components/ChartDialog.jsx';
import ShapeGallery from '../components/ShapeGallery.jsx';
import TablePicker from '../components/TablePicker.jsx';
import ImageDialog from '../components/ImageDialog.jsx';
import {
  Ribbon, Group, Btn, Select, NumInput, ColorPicker, Check, Popover,
  ContextMenu, useContextMenu, ZoomSlider,
} from '../components/ui.jsx';
import SlideCanvas from './SlideCanvas.jsx';
import SlideSorter from './SlideSorter.jsx';
import Slideshow from './Slideshow.jsx';
import { toggleWrap, toggleLinePrefix } from '../lib/markdown.js';
import { alignBlocks, distributeBlocks } from './blockOps.js';

const TABS = ['파일', '홈', '삽입', '디자인', '슬라이드 쇼', 'AI'];
/** Tabs that exist only while something is selected. */
const CONTEXT_TABS = ['도형 서식', '표 디자인', '레이아웃'];

const LINE_DASHES = [
  { value: 'solid', label: '실선' },
  { value: 'dash', label: '파선' },
  { value: 'dot', label: '점선' },
  { value: 'dashDot', label: '일점쇄선' },
  { value: 'longDash', label: '긴 파선' },
];
const LINE_WIDTHS = [
  { value: '0.75', label: '¼ pt' },
  { value: '1', label: '1 pt' },
  { value: '2', label: '2 pt' },
  { value: '3', label: '3 pt' },
  { value: '4.5', label: '4½ pt' },
  { value: '6', label: '6 pt' },
];
/** The size list the box offers, which Ctrl+Shift+> steps through. */
const FONT_SIZES = [8, 10, 12, 14, 16, 18, 20, 24, 28, 32, 36, 40, 44, 54, 60, 66, 72, 80, 96, 120];

/** Office's paragraph-alignment shortcuts, in every app that has paragraphs. */
const PARA_ALIGN_KEYS = { l: 'left', e: 'center', r: 'right', j: 'justify' };

/** PowerPoint's own 줄 간격 list. */
const LINE_SPACINGS = [
  { value: '1', label: '1.0' },
  { value: '1.15', label: '1.15' },
  { value: '1.45', label: '1.45' },
  { value: '1.5', label: '1.5' },
  { value: '2', label: '2.0' },
  { value: '2.5', label: '2.5' },
  { value: '3', label: '3.0' },
];
const TABLE_STYLES = [
  { value: 'banded', label: '줄무늬' },
  { value: 'plain', label: '격자' },
  { value: 'borderless', label: '테두리 없음' },
];

/** `A1`-style address for a table cell, matching the layout JSON. */
function cellRef(col, row) {
  let n = Math.max(0, col) + 1;
  let name = '';
  while (n > 0) {
    name = String.fromCharCode(65 + ((n - 1) % 26)) + name;
    n = Math.floor((n - 1) / 26);
  }
  return `${name}${row + 1}`;
}

function cellFormatOf(block, cell) {
  if (!block || !cell || cell.id !== block.id) return null;
  return block.table?.cells?.[cellRef(cell.col, cell.row)] ?? null;
}

/** The shape's name for the ribbon, falling back to the raw preset. */
function shapeLabel(shape) {
  return shape?.preset ?? '';
}

/** A table's size, read from its markdown. */
function tableSize(block) {
  const info = tableInfo(block?.md ?? '', block?.table ?? null);
  return { columns: info.columns, rows: info.rows };
}
const LAYOUT_LABELS = {
  title: '표지',
  'title-content': '제목 + 내용',
  'two-column': '2단',
  section: '섹션 구분',
  blank: '빈 슬라이드',
};

/**
 * The slide sizes PowerPoint offers, plus whatever this deck actually is.
 *
 * An imported deck can be any size at all — a handout template is A4-shaped, a
 * banner is square, and an author who typed their own size meant it. Dropping it
 * from the list would make the dropdown show something the slide is not, and
 * picking that entry back would silently resize the whole deck.
 *
 * The px sizes are PowerPoint's own paper sizes at 96dpi: 13.333×7.5in for
 * widescreen, 10×7.5in for standard, and A4/Letter as PowerPoint lays them out
 * for printed handouts.
 */
const CANVAS_SIZES = [
  { value: '1280x720', label: '와이드스크린 (16:9)' },
  { value: '960x720', label: '표준 (4:3)' },
  { value: '1058x744', label: 'A4 (가로)' },
  { value: '744x1058', label: 'A4 (세로)' },
  { value: '1056x816', label: 'Letter (가로)' },
  { value: '720x720', label: '정사각형 (1:1)' },
];

function canvasSizes(canvas) {
  const current = `${canvas.w}x${canvas.h}`;
  if (CANVAS_SIZES.some((o) => o.value === current)) return CANVAS_SIZES;
  return [...CANVAS_SIZES, { value: current, label: `사용자 지정 (${canvas.w}×${canvas.h})` }];
}

/** The ratio a size reduces to, for the label beside the size box. */
function ratioOf(w, h) {
  const gcd = (a, b) => (b < 1 ? a : gcd(b, a % b));
  const divisor = gcd(Math.round(w), Math.round(h)) || 1;
  const [rw, rh] = [Math.round(w) / divisor, Math.round(h) / divisor];
  // 1280x720 reduces to 16:9; 1058x744 reduces to nothing useful, so a decimal
  // ratio is the honest label there.
  return rw <= 40 && rh <= 40 ? `${rw}:${rh}` : `${(w / h).toFixed(2)} : 1`;
}

export default function DeckEditor({ ctl, onHome, notify, onNewProject }) {
  const { project, items: slides, setItems, setTitle, save, saving, dirty, savedAt, undo, redo, canUndo, canRedo } = ctl;

  const [current, setCurrent] = useState(0);
  const [selectedId, setSelectedId] = useState(null);
  const [editingId, setEditingId] = useState(null);
  const [tab, setTab] = useState('홈');
  const [zoomMode, setZoomMode] = useState('fit');
  const [zoom, setZoom] = useState(1);
  const [inspectorOpen, setInspectorOpen] = useState(true);
  const [stageSize, setStageSize] = useState({ w: 900, h: 560 });
  const [imageDialog, setImageDialog] = useState(null);
  const [chartDialog, setChartDialog] = useState(null);
  const [shapePicker, setShapePicker] = useState(false);
  const [tablePicker, setTablePicker] = useState(false);
  // The controls the pickers hang from. They are positioned against these boxes
  // rather than nested inside them, so the ribbon cannot clip them.
  const shapeAnchor = useRef(null);
  const tableAnchor = useRef(null);
  // A shape armed from the gallery: the next drag on the canvas draws it.
  const [pendingShape, setPendingShape] = useState(null);
  // Which cell of the selected table has the cursor.
  const [activeCell, setActiveCell] = useState(null);
  // Office remembers the last shapes you used, at the top of the gallery.
  const [recentShapes, setRecentShapes] = useState([]);
  const [presenting, setPresenting] = useState(false);
  const [clipboard, setClipboard] = useState(null);
  const ctx = useContextMenu();

  const slideIndex = Math.min(current, Math.max(0, slides.length - 1));
  const slide = slides[slideIndex];

  useEffect(() => {
    setSelectedId(null);
    setEditingId(null);
  }, [slideIndex]);

  /* ------------------------------------------------------------ mutations */

  const patchSlide = useCallback(
    (updater, options) => setItems((list) => list.map((s, i) => (i === slideIndex ? updater(s) : s)), options),
    [setItems, slideIndex]
  );

  const patchBlock = useCallback(
    (blockId, patch, options) =>
      patchSlide(
        (s) => ({ ...s, blocks: s.blocks.map((b) => (b.id === blockId ? { ...b, ...patch } : b)) }),
        options
      ),
    [patchSlide]
  );

  const patchStyle = useCallback(
    (blockId, styleDelta) =>
      patchSlide((s) => ({
        ...s,
        blocks: s.blocks.map((b) => (b.id === blockId ? { ...b, style: { ...b.style, ...styleDelta } } : b)),
      })),
    [patchSlide]
  );

  const changeBlockBox = useCallback(
    (blockId, box, { merge } = {}) => patchBlock(blockId, box, merge ? { mergeKey: `nudge:${blockId}` } : undefined),
    [patchBlock]
  );

  const changeBlockMd = useCallback(
    (blockId, md) => patchBlock(blockId, { md }, { mergeKey: `text:${blockId}` }),
    [patchBlock]
  );

  const addBlock = useCallback(
    (partial = {}) => {
      const id = newBlockId();
      const maxZ = Math.max(0, ...(slide?.blocks ?? []).map((b) => b.z ?? 0));
      const block = {
        id,
        kind: 'text',
        md: '새 텍스트',
        x: 200,
        y: 200,
        w: 420,
        h: 90,
        z: maxZ + 1,
        style: { fontSize: 20, weight: 400, align: 'left', color: '#1f2937', lineHeight: 1.45 },
        ...partial,
      };
      patchSlide((s) => ({ ...s, blocks: [...s.blocks, block] }));
      setSelectedId(id);
      if ((partial.kind ?? 'text') === 'text') setEditingId(id);
      return id;
    },
    [patchSlide, slide]
  );

  /** Where a newly inserted object goes: roughly centred, as Office does it. */
  const insertionBox = useCallback(
    (w, h) => {
      const canvas = slide?.canvas ?? { w: 1280, h: 720 };
      const maxZ = Math.max(0, ...(slide?.blocks ?? []).map((b) => b.z ?? 0));
      return {
        x: Math.round((canvas.w - w) / 2),
        y: Math.round((canvas.h - h) / 2),
        w,
        h,
        z: maxZ + 1,
      };
    },
    [slide]
  );

  /**
   * Pick a shape from the gallery.
   *
   * Office does not insert it immediately: the cursor becomes a crosshair and
   * the next drag on the slide draws the shape at whatever size you sweep. A
   * plain click drops it at the default size. Both land in `onDrawShape`.
   */
  const insertShape = useCallback((preset) => {
    setPendingShape(preset);
    setShapePicker(false);
    // The gallery keeps the last few, newest first, the way Office does.
    setRecentShapes((prev) =>
      [preset, ...prev.filter((p) => p.name !== preset.name)].slice(0, 10)
    );
  }, []);

  /** Escape disarms a pending shape, as it does in Office. */
  useEffect(() => {
    if (!pendingShape) return undefined;
    const onKey = (e) => e.key === 'Escape' && setPendingShape(null);
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [pendingShape]);

  const nextZ = useCallback(
    () => Math.max(0, ...(slide?.blocks ?? []).map((b) => b.z ?? 0)) + 1,
    [slide]
  );

  const insertTable = useCallback(
    (columns, rows) => {
      // Office sizes a new table by its content: ~120px a column, ~32px a row.
      const w = Math.min(1100, Math.max(240, columns * 120));
      const h = Math.min(600, Math.max(80, rows * 34));
      const block = makeTable(columns, rows, insertionBox(w, h));
      patchSlide((s) => ({ ...s, blocks: [...s.blocks, block] }));
      setSelectedId(block.id);
      setActiveCell({ id: block.id, col: 0, row: 0, editing: false });
      setTablePicker(false);
      setTab('표 디자인');
      return block.id;
    },
    [insertionBox, patchSlide]
  );

  /** Patch the selected shape's spec, leaving the rest of the block alone. */
  const patchShape = useCallback(
    (blockId, partial) => {
      const block = (slide?.blocks ?? []).find((b) => b.id === blockId);
      if (!block) return;
      patchBlock(blockId, { shape: { ...(block.shape ?? {}), ...partial } });
    },
    [patchBlock, slide]
  );

  /** Patch the selected table's layout. */
  const patchTable = useCallback(
    (blockId, partial) => {
      const block = (slide?.blocks ?? []).find((b) => b.id === blockId);
      if (!block) return;
      patchBlock(blockId, { table: { ...(block.table ?? {}), ...partial } });
    },
    [patchBlock, slide]
  );

  /**
   * One structural edit to the selected table, applied by the Rust core.
   *
   * The markdown and the layout change together — inserting a column shifts the
   * merges — so both come back from one call rather than being patched apart.
   */
  const editTable = useCallback(
    (op, args = {}) => {
      const block = (slide?.blocks ?? []).find((b) => b.id === selectedId);
      if (!block || block.kind !== 'table') return;
      const next = tableEdit(block.md, block.table, op, args);
      patchBlock(block.id, { md: next.md, table: next.table });
    },
    [patchBlock, selectedId, slide]
  );

  const deleteBlock = useCallback(
    (blockId) => {
      patchSlide((s) => ({ ...s, blocks: s.blocks.filter((b) => b.id !== blockId) }));
      setSelectedId(null);
      setEditingId(null);
    },
    [patchSlide]
  );

  const duplicateBlock = useCallback(
    (blockId) => {
      const block = slide?.blocks.find((b) => b.id === blockId);
      if (!block) return;
      const id = newBlockId();
      const maxZ = Math.max(0, ...slide.blocks.map((b) => b.z ?? 0));
      patchSlide((s) => ({
        ...s,
        blocks: [...s.blocks, { ...block, id, x: block.x + 16, y: block.y + 16, z: maxZ + 1 }],
      }));
      setSelectedId(id);
    },
    [patchSlide, slide]
  );

  /**
   * Move a block one step through the stack.
   *
   * Office's 앞으로 가져오기 swaps the block with its nearest neighbour rather
   * than jumping to the front — with three overlapping shapes, only the swap
   * gets you the middle of the three.
   */
  const bumpZ = useCallback(
    (blockId, direction) => {
      const ordered = [...(slide?.blocks ?? [])].sort((a, b) => (a.z ?? 0) - (b.z ?? 0));
      const at = ordered.findIndex((b) => b.id === blockId);
      const swap = ordered[at + direction];
      if (at < 0 || !swap) return;
      const mine = ordered[at].z ?? 0;
      patchSlide((s) => ({
        ...s,
        blocks: s.blocks.map((b) =>
          b.id === blockId ? { ...b, z: swap.z ?? 0 } : b.id === swap.id ? { ...b, z: mine } : b
        ),
      }));
    },
    [patchSlide, slide]
  );

  const transformMd = useCallback(
    (transform) => {
      if (!selectedId) return;
      const block = slide?.blocks.find((b) => b.id === selectedId);
      if (!block) return;
      const md = String(block.md ?? '');
      changeBlockMd(selectedId, transform(md, 0, md.length).value);
    },
    [selectedId, slide, changeBlockMd]
  );

  /* ------------------------------------------------------ block clipboard */

  const copyBlock = useCallback(
    (blockId, cut) => {
      const block = slide?.blocks.find((b) => b.id === blockId);
      if (!block) return;
      setClipboard(block);
      if (cut) deleteBlock(blockId);
      notify(cut ? '요소를 잘라냈습니다' : '요소를 복사했습니다');
    },
    [slide, deleteBlock, notify]
  );

  const pasteBlock = useCallback(() => {
    if (!clipboard) return;
    const id = newBlockId();
    const maxZ = Math.max(0, ...(slide?.blocks ?? []).map((b) => b.z ?? 0));
    patchSlide((s) => ({
      ...s,
      blocks: [...s.blocks, { ...clipboard, id, x: clipboard.x + 24, y: clipboard.y + 24, z: maxZ + 1 }],
    }));
    setSelectedId(id);
  }, [clipboard, patchSlide, slide]);

  /* -------------------------------------------------------- format painter */

  /**
   * 서식 복사 — carry one object's look to another.
   *
   * Text style always, plus a shape's fill and outline when both ends are
   * shapes; geometry and content never move. This is how a slide full of boxes
   * gets made consistent without setting six controls per box.
   */
  const [painter, setPainter] = useState(null);

  const pickUpFormat = useCallback(() => {
    if (painter) {
      setPainter(null);
      return;
    }
    const block = (slide?.blocks ?? []).find((b) => b.id === selectedId);
    if (!block) return;
    setPainter({
      style: block.style ? { ...block.style } : null,
      shape: block.shape ? { fill: block.shape.fill, line: block.shape.line } : null,
    });
    notify('서식을 집었습니다 — 붙일 개체를 누르세요');
  }, [painter, slide, selectedId, notify]);

  const paintOnto = useCallback(
    (blockId) => {
      if (!painter) return;
      const block = (slide?.blocks ?? []).find((b) => b.id === blockId);
      if (!block) return;
      const patch = { style: painter.style ? { ...painter.style } : {} };
      // Fill and outline only between shapes; a text box has no geometry to fill.
      if (block.kind === 'shape' && painter.shape) {
        patch.shape = { ...(block.shape ?? {}), fill: painter.shape.fill, line: painter.shape.line };
      }
      patchBlock(blockId, patch);
      setPainter(null);
      notify('서식을 붙였습니다');
    },
    [painter, slide, patchBlock, notify]
  );

  /* --------------------------------------------------------- slide commands */

  const addSlide = (layoutName = 'title-content') => {
    const next = makeSlide(layoutName, { index: slides.length + 1 });
    setItems((list) => [...list.slice(0, slideIndex + 1), next, ...list.slice(slideIndex + 1)]);
    setCurrent(slideIndex + 1);
  };

  /**
   * Duplicate one slide.
   *
   * The index is a parameter rather than read from `slideIndex`, because the
   * right-click menu duplicates the slide that was clicked — and a `setCurrent(at)`
   * right before this call has not been applied yet, so reading the current
   * index would copy whichever slide happened to be open.
   */
  const duplicateSlide = (at = slideIndex) => {
    const source = slides[at];
    if (!source) return;
    const copy = {
      ...source,
      id: undefined,
      title: `${source.title} 사본`,
      blocks: source.blocks.map((b) => ({ ...b, id: newBlockId() })),
    };
    setItems((list) => [...list.slice(0, at + 1), copy, ...list.slice(at + 1)]);
    setCurrent(at + 1);
  };

  const deleteSlide = (at = slideIndex) => {
    if (slides.length <= 1) {
      notify('마지막 슬라이드는 삭제할 수 없습니다');
      return;
    }
    setItems((list) => list.filter((_, i) => i !== at));
    setCurrent(Math.max(0, at - 1));
  };

  const moveSlide = (from, to) => {
    if (to < 0 || to >= slides.length || from === to) return;
    setItems((list) => {
      const next = [...list];
      const [moved] = next.splice(from, 1);
      next.splice(to, 0, moved);
      return next;
    });
    setCurrent(to);
  };

  const relayout = () => {
    patchSlide((s) => ({
      ...s,
      blocks: s.blocks.map((b, i) => ({ ...b, ...autoLayout(i, s.blocks.length, s.canvas) })),
    }));
    notify('블록을 자동 정렬했습니다');
  };

  /* -------------------------------------------------------- align helpers */

  const doAlign = useCallback(
    (edge) => {
      patchSlide((s) => ({ ...s, blocks: alignBlocks(s.blocks, edge, s.canvas, selectedId) }));
    },
    [patchSlide, selectedId]
  );

  const doDistribute = useCallback(
    (axis) => {
      patchSlide((s) => ({ ...s, blocks: distributeBlocks(s.blocks, axis) }));
    },
    [patchSlide]
  );

  /* ---------------------------------------------------------------- keys */

  useEffect(() => {
    const onKey = (e) => {
      if (presenting) return;
      const tag = e.target?.tagName;
      // The block's own textarea is not a stray form field: PowerPoint's
      // paragraph shortcuts are meant to work with the caret inside a text box.
      const inBlockEditor = !!e.target?.classList?.contains?.('block__editor');
      if ((tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') && !inBlockEditor) return;
      const mod = e.metaKey || e.ctrlKey;

      // Ctrl+L / E / R / J — 왼쪽 · 가운데 · 오른쪽 · 양쪽 맞춤.
      if (mod && !e.shiftKey && !e.altKey && selectedId && PARA_ALIGN_KEYS[e.key.toLowerCase()]) {
        e.preventDefault();
        patchStyle(selectedId, { align: PARA_ALIGN_KEYS[e.key.toLowerCase()] });
        return;
      }
      // Ctrl+Shift+> / < — 글자 크기 한 단계, as in PowerPoint.
      if (mod && e.shiftKey && selectedId && ['>', '<', '.', ','].includes(e.key)) {
        e.preventDefault();
        const up = e.key === '>' || e.key === '.';
        const current = (slide?.blocks ?? []).find((b) => b.id === selectedId)?.style?.fontSize ?? 20;
        const at = FONT_SIZES.findIndex((n) => n >= current);
        const from = at < 0 ? FONT_SIZES.length - 1 : at;
        patchStyle(selectedId, {
          fontSize: FONT_SIZES[Math.min(FONT_SIZES.length - 1, Math.max(0, from + (up ? 1 : -1)))],
        });
        return;
      }
      if (inBlockEditor) return;

      // PowerPoint: F5 runs the deck from the top, Shift+F5 from the slide you
      // are looking at. Starting from the current slide on a bare F5 is a
      // surprise when you have scrolled to slide 14 to fix a typo.
      if (e.key === 'F5') {
        e.preventDefault();
        if (!e.shiftKey) setCurrent(0);
        setPresenting(true);
        return;
      }
      // Ctrl+M is how a new slide gets made in PowerPoint.
      if (mod && e.key.toLowerCase() === 'm') {
        e.preventDefault();
        addSlide();
        return;
      }
      /*
       * Tab walks the objects on the slide, Shift+Tab walks back.
       *
       * PowerPoint users press Tab on an empty slide to find the placeholders
       * and again to step through them; without it the only way to reach a block
       * behind another one is to move the one in front out of the way.
       */
      // A table with a cell cursor owns Tab — it walks the cells.
      if (e.key === 'Tab' && activeCell?.id === selectedId) return;
      if (e.key === 'Tab' && (slide?.blocks?.length ?? 0) > 0) {
        e.preventDefault();
        const order = readingOrder(slide.blocks).map((b) => b.id);
        const at = order.indexOf(selectedId);
        const step = e.shiftKey ? -1 : 1;
        const next = order[(at + step + order.length) % order.length];
        setEditingId(null);
        setSelectedId(next);
        return;
      }
      if (mod && e.key.toLowerCase() === 'd' && selectedId) {
        e.preventDefault();
        duplicateBlock(selectedId);
        return;
      }
      if (mod && e.key.toLowerCase() === 'c' && selectedId) {
        e.preventDefault();
        copyBlock(selectedId, false);
        return;
      }
      if (mod && e.key.toLowerCase() === 'x' && selectedId) {
        e.preventDefault();
        copyBlock(selectedId, true);
        return;
      }
      if (mod && e.key.toLowerCase() === 'v' && clipboard) {
        e.preventDefault();
        pasteBlock();
        return;
      }
      if (e.key === 'PageDown') {
        e.preventDefault();
        setCurrent((i) => Math.min(slides.length - 1, i + 1));
        return;
      }
      if (e.key === 'PageUp') {
        e.preventDefault();
        setCurrent((i) => Math.max(0, i - 1));
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [presenting, selectedId, activeCell, clipboard, duplicateBlock, copyBlock, pasteBlock, slides.length, slide, slideIndex, patchStyle]);

  /**
   * Resize every slide in the deck.
   *
   * PowerPoint asks whether to scale the content when the size changes; this
   * keeps the blocks where they are and only moves what would fall off the new
   * canvas, which is the "맞춤 확인" answer — nothing gets scaled to a size the
   * author did not choose.
   */
  const resizeCanvas = useCallback(
    (w, h) => {
      const canvas = { w: Math.max(200, Math.round(w)), h: Math.max(200, Math.round(h)) };
      setItems((list) =>
        list.map((s) => ({
          ...s,
          canvas: { ...s.canvas, ...canvas },
          blocks: s.blocks.map((b) => ({ ...b, ...clampBox(b, { ...s.canvas, ...canvas }) })),
        }))
      );
    },
    [setItems]
  );

  /* ------------------------------------------------------------------ zoom */

  const scale = useMemo(() => {
    if (!slide) return 1;
    if (zoomMode !== 'fit') return zoom;
    const pad = 60;
    return Math.min(
      1.6,
      Math.max(0.2, Math.min((stageSize.w - pad) / slide.canvas.w, (stageSize.h - pad) / slide.canvas.h))
    );
  }, [zoomMode, zoom, stageSize, slide]);

  const stageRef = useCallback((node) => {
    if (!node) return;
    const observer = new ResizeObserver(([entry]) => {
      setStageSize({ w: entry.contentRect.width, h: entry.contentRect.height });
    });
    observer.observe(node);
  }, []);

  const selected = slide?.blocks.find((b) => b.id === selectedId) ?? null;

  const openBlock = useCallback(
    (block) => {
      if (block.kind === 'chart') {
        setChartDialog({ id: block.id, spec: parseChartBlock(block.md) });
      } else if (block.kind === 'image') {
        const m = String(block.md ?? '').match(/!\[([^\]]*)\]\(([^)\s]+)/);
        setImageDialog({ id: block.id, alt: m?.[1] ?? '', src: m?.[2] ?? '' });
      }
    },
    []
  );

  if (!slide) return <div className="center-note">슬라이드를 불러오는 중…</div>;

  /* --------------------------------------------------------- context menu */

  const blockMenu = (block) => [
    { label: '잘라내기', shortcut: 'Ctrl+X', onClick: () => copyBlock(block.id, true) },
    { label: '복사', shortcut: 'Ctrl+C', onClick: () => copyBlock(block.id, false) },
    { label: '붙여넣기', shortcut: 'Ctrl+V', disabled: !clipboard, onClick: pasteBlock },
    { label: '복제', shortcut: 'Ctrl+D', onClick: () => duplicateBlock(block.id) },
    ...(painter ? [{ label: '집어 둔 서식 붙이기', onClick: () => paintOnto(block.id) }] : []),
    '-',
    ...(block.kind === 'chart' || block.kind === 'image'
      ? [{ label: block.kind === 'chart' ? '차트 편집' : '이미지 편집', onClick: () => openBlock(block) }]
      : [{ label: '텍스트 편집', shortcut: 'Enter', onClick: () => setEditingId(block.id) }]),
    '-',
    { label: '맨 앞으로 가져오기', onClick: () => patchBlock(block.id, { z: maxZ(slide) + 1 }) },
    { label: '앞으로 가져오기', onClick: () => bumpZ(block.id, 1) },
    { label: '뒤로 보내기', onClick: () => bumpZ(block.id, -1) },
    { label: '맨 뒤로 보내기', onClick: () => patchBlock(block.id, { z: minZ(slide) - 1 }) },
    { label: block.locked ? '잠금 해제' : '위치 잠금', onClick: () => patchBlock(block.id, { locked: !block.locked }) },
    '-',
    { label: '삭제', shortcut: 'Delete', danger: true, onClick: () => deleteBlock(block.id) },
  ];

  const canvasMenu = () => [
    { label: '텍스트 상자 추가', onClick: () => addBlock({ md: '새 텍스트' }) },
    { label: '이미지 추가', onClick: () => setImageDialog({ src: '', alt: '' }) },
    { label: '차트 추가', onClick: () => setChartDialog({ spec: null }) },
    '-',
    { label: '붙여넣기', shortcut: 'Ctrl+V', disabled: !clipboard, onClick: pasteBlock },
    { label: '블록 자동 정렬', onClick: relayout },
  ];

  const slideMenu = (at) => [
    { label: '새 슬라이드', onClick: () => addSlide() },
    { label: '복제', onClick: () => duplicateSlide(at) },
    '-',
    { label: '위로 이동', disabled: at === 0, onClick: () => moveSlide(at, at - 1) },
    { label: '아래로 이동', disabled: at === slides.length - 1, onClick: () => moveSlide(at, at + 1) },
    '-',
    { label: '여기서 슬라이드 쇼', onClick: () => { setCurrent(at); setPresenting(true); } },
    '-',
    { label: '삭제', danger: true, disabled: slides.length <= 1, onClick: () => deleteSlide(at) },
  ];

  /* ---------------------------------------------------------------- ribbon */

  /** The cursor, but only when it belongs to the selected table. */
  const cell = activeCell?.id === selected?.id ? activeCell : null;

  /* Office shows a contextual tab set for whatever is selected, and takes it
     away again when the selection changes. */
  const contextual =
    selected?.kind === 'shape'
      ? { label: '도형 도구', tabs: ['도형 서식'] }
      : selected?.kind === 'table'
      ? { label: '표 도구', tabs: ['표 디자인', '레이아웃'] }
      : { label: null, tabs: [] };

  // A tab that belongs to a selection that is gone must not stay open.
  useEffect(() => {
    if (CONTEXT_TABS.includes(tab) && !contextual.tabs.includes(tab)) setTab('홈');
  }, [tab, contextual.tabs]);

  /** Rotate by a quarter turn, the way Office's rotate menu does. */
  const rotateBy = (block, degrees) => {
    const next = (((block.shape?.rotation ?? 0) + degrees) % 360 + 360) % 360;
    patchShape(block.id, { rotation: next });
  };

  /**
   * Column widths and row heights.
   *
   * The layout stores a sparse list, so a width has to be written at the right
   * index with the columns before it filled in — otherwise setting column 3's
   * width would silently become column 1's.
   */
  const colWidthOf = (block, cell) => {
    if (!cell) return 0;
    const columns = tableSize(block).columns;
    const cols = block.table?.cols ?? [];
    const assigned = cols[cell.col];
    if (assigned > 0) return assigned;
    const known = cols.slice(0, columns).filter((w) => w > 0);
    const rest = columns - known.length;
    return rest > 0 ? Math.max(24, (block.w - known.reduce((a, b) => a + b, 0)) / rest) : block.w / columns;
  };

  const rowHeightOf = (block, cell) => {
    if (!cell) return 0;
    const rows = tableSize(block).rows;
    const stored = block.table?.rows?.[cell.row];
    return stored > 0 ? stored : block.h / Math.max(1, rows);
  };

  const setColWidth = (block, cell, width) => {
    if (!cell || !(width > 0)) return;
    const columns = tableSize(block).columns;
    const cols = Array.from({ length: columns }, (_, i) =>
      i === cell.col ? Math.max(24, Math.round(width)) : block.table?.cols?.[i] ?? Math.round(colWidthOf(block, { col: i, row: 0 }))
    );
    patchTable(block.id, { cols });
  };

  const setRowHeight = (block, cell, height) => {
    if (!cell || !(height > 0)) return;
    const rows = tableSize(block).rows;
    const list = Array.from({ length: rows }, (_, i) =>
      i === cell.row ? Math.max(16, Math.round(height)) : block.table?.rows?.[i] ?? Math.round(rowHeightOf(block, { col: 0, row: i }))
    );
    patchTable(block.id, { rows: list });
  };

  const ribbon = (
    <Ribbon
      tabs={TABS}
      active={tab}
      onTab={setTab}
      contextual={contextual.tabs}
      contextLabel={contextual.label}
    >
      {tab === '파일' && (
        <FileMenu
          project={project}
          dirty={dirty}
          onSave={save}
          onHome={onHome}
          onNewProject={onNewProject}
          notify={notify}
        />
      )}

      {tab === '홈' && (
        <>
          <Group label="되돌리기">
            <Btn icon="↶" label="실행 취소" onClick={undo} disabled={!canUndo} title="Ctrl+Z" />
            <Btn icon="↷" label="다시 실행" onClick={redo} disabled={!canRedo} title="Ctrl+Shift+Z" />
          </Group>

          <Group label="클립보드">
            <Btn icon="📋" label="붙여넣기" onClick={pasteBlock} disabled={!clipboard} title="Ctrl+V" />
            <div className="rcol">
              <Btn small icon="⧉" label="복사" disabled={!selected} onClick={() => copyBlock(selectedId, false)} title="Ctrl+C" />
              <Btn small icon="✂" label="잘라내기" disabled={!selected} onClick={() => copyBlock(selectedId, true)} title="Ctrl+X" />
              <Btn
                small
                icon="🖌"
                label={painter ? '집은 상태' : '서식 복사'}
                title="글자 서식과 도형 채우기·윤곽선만 다른 개체로 옮깁니다"
                pressed={!!painter}
                disabled={!selected && !painter}
                onClick={pickUpFormat}
              />
            </div>
          </Group>

          <Group label="슬라이드">
            <Btn icon="🞤" label="새 슬라이드" onClick={() => addSlide()} />
            <Btn icon="⧉" label="복제" onClick={() => duplicateSlide()} />
            <Btn icon="🗑" label="삭제" onClick={() => deleteSlide()} disabled={slides.length <= 1} />
            <div className="rcol">
              <Select
                value={slide.layoutName}
                onChange={(v) => patchSlide((s) => ({ ...s, layoutName: v }))}
                options={SLIDE_LAYOUTS.map((l) => ({ value: l, label: LAYOUT_LABELS[l] ?? l }))}
                title="레이아웃"
                width={124}
              />
              <Btn small icon="⇕" label="자동 정렬" onClick={relayout} />
            </div>
          </Group>

          <Group label="글꼴">
            <div className="rcol">
              <div className="rrow">
                <NumInput
                  value={selected?.style?.fontSize ?? 20}
                  onChange={(v) => selected && patchStyle(selected.id, { fontSize: v })}
                  title="글자 크기"
                  min={8}
                  max={140}
                />
                <Btn
                  small
                  label="A+"
                  title="크게"
                  disabled={!selected}
                  onClick={() => patchStyle(selected.id, { fontSize: (selected.style?.fontSize ?? 20) + 4 })}
                />
                <Btn
                  small
                  label="A−"
                  title="작게"
                  disabled={!selected}
                  onClick={() => patchStyle(selected.id, { fontSize: Math.max(8, (selected.style?.fontSize ?? 20) - 4) })}
                />
              </div>
              <div className="rrow">
                <Btn small icon="B" label="" title="굵게 (마크다운 **)" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleWrap(md, a, b, '**'))} />
                <Btn small icon="I" label="" title="기울임 (마크다운 *)" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleWrap(md, a, b, '*'))} />
                <Btn small label="H1" title="제목 1 (#)" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleLinePrefix(md, a, b, '# '))} />
                <Btn small label="H2" title="제목 2 (##)" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleLinePrefix(md, a, b, '## '))} />
                <Btn small icon="•" label="" title="목록 (-)" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleLinePrefix(md, a, b, '- '))} />
              </div>
            </div>
          </Group>

          <Group label="색">
            <div className="rcol">
              <ColorPicker
                value={selected?.style?.color}
                onChange={(c) => selected && patchStyle(selected.id, { color: c })}
                title="글자 색"
                none="자동"
                accent={project.manifest?.theme?.accent}
                disabled={!selected}
              />
              <ColorPicker
                value={selected?.style?.fill}
                onChange={(c) => selected && patchStyle(selected.id, { fill: c })}
                title="채우기 색"
                none="채우기 없음"
                accent={project.manifest?.theme?.accent}
                disabled={!selected}
              />
            </div>
          </Group>

          <Group label="단락">
            <div className="rcol">
              <div className="rrow">
                {[
                  ['left', '⬅', '왼쪽'],
                  ['center', '↔', '가운데'],
                  ['right', '➡', '오른쪽'],
                ].map(([value, icon, title]) => (
                  <Btn key={value} small icon={icon} label="" title={`문단 ${title} 정렬 (Ctrl+${{ left: 'L', center: 'E', right: 'R' }[value]})`} disabled={!selected} pressed={selected?.style?.align === value} onClick={() => patchStyle(selected.id, { align: value })} />
                ))}
                {/* PowerPoint's 줄 간격 sits in this group, and the exporter
                    already writes it as `a:lnSpc` — so it survives the round
                    trip to a real .pptx. */}
                <Select
                  value={String(selected?.style?.lineHeight ?? 1.45)}
                  onChange={(v) => selected && patchStyle(selected.id, { lineHeight: Number(v) })}
                  options={LINE_SPACINGS}
                  title="줄 간격"
                  width={74}
                />
              </div>
              <div className="rrow">
                {[
                  ['top', '⤒', '위'],
                  ['middle', '⇳', '중간'],
                  ['bottom', '⤓', '아래'],
                ].map(([value, icon, title]) => (
                  <Btn key={value} small icon={icon} label="" title={`텍스트 세로 ${title}`} disabled={!selected} pressed={selected?.style?.valign === value} onClick={() => patchStyle(selected.id, { valign: value })} />
                ))}
              </div>
            </div>
          </Group>

          <Group label="그리기">
            <div className="rcol">
              <div className="rrow">
                {[
                  ['left', '⇤', '왼쪽 맞춤'],
                  ['hcenter', '⇔', '가로 가운데'],
                  ['right', '⇥', '오른쪽 맞춤'],
                  ['top', '⤒', '위 맞춤'],
                  ['vcenter', '⇕', '세로 가운데'],
                  ['bottom', '⤓', '아래 맞춤'],
                ].map(([edge, icon, title]) => (
                  <Btn key={edge} small icon={icon} label="" title={`개체 ${title}`} disabled={!selected} onClick={() => doAlign(edge)} />
                ))}
              </div>
              <div className="rrow">
                <Btn small icon="⤒" label="맨 앞으로" title="맨 앞으로 가져오기" disabled={!selected} onClick={() => patchBlock(selected.id, { z: maxZ(slide) + 1 })} />
                <Btn small icon="⬆" label="앞으로" title="한 단계 앞으로 가져오기" disabled={!selected} onClick={() => bumpZ(selected.id, 1)} />
                <Btn small icon="⬇" label="뒤로" title="한 단계 뒤로 보내기" disabled={!selected} onClick={() => bumpZ(selected.id, -1)} />
                <Btn small icon="⤓" label="맨 뒤로" title="맨 뒤로 보내기" disabled={!selected} onClick={() => patchBlock(selected.id, { z: minZ(slide) - 1 })} />
                <Btn small icon="⋮" label="세로 분배" disabled={slide.blocks.length < 3} onClick={() => doDistribute('vertical')} />
                <Btn small icon="⋯" label="가로 분배" disabled={slide.blocks.length < 3} onClick={() => doDistribute('horizontal')} />
              </div>
            </div>
          </Group>
        </>
      )}

      {tab === '삽입' && (
        <>
          <Group label="표">
            <div className="ribbon__popholder" ref={tableAnchor}>
              <Btn
                icon="▦"
                label="표"
                pressed={tablePicker}
                onClick={() => {
                  setTablePicker((v) => !v);
                  setShapePicker(false);
                }}
              />
              {tablePicker && (
                <Popover anchorRef={tableAnchor} onClose={() => setTablePicker(false)} label="표 삽입">
                  <TablePicker
                    onPick={(columns, rows) => insertTable(columns, rows)}
                    onClose={() => setTablePicker(false)}
                  />
                </Popover>
              )}
            </div>
          </Group>

          <Group label="이미지">
            <Btn icon="🖼" label="그림" onClick={() => setImageDialog({ src: '', alt: '' })} />
          </Group>

          <Group label="일러스트레이션">
            <div className="ribbon__popholder" ref={shapeAnchor}>
              <Btn
                icon="◇"
                label="도형"
                pressed={shapePicker}
                onClick={() => {
                  setShapePicker((v) => !v);
                  setTablePicker(false);
                }}
              />
              {shapePicker && (
                <Popover anchorRef={shapeAnchor} onClose={() => setShapePicker(false)} label="도형">
                <ShapeGallery
                  recent={recentShapes}
                  onPick={(preset) => insertShape(preset)}
                  onClose={() => setShapePicker(false)}
                />
                </Popover>
              )}
            </div>
            <Btn icon="📊" label="차트" onClick={() => setChartDialog({ spec: null })} />
          </Group>

          <Group label="텍스트">
            <Btn icon="T" label="텍스트 상자" onClick={() => addBlock({ md: '새 텍스트' })} />
            <Btn icon="H" label="제목" onClick={() => addBlock({ md: '# 제목', style: { fontSize: 36, weight: 700, align: 'left' }, h: 80 })} />
            <Btn icon="≔" label="목록" onClick={() => addBlock({ md: '- 항목 1\n- 항목 2', h: 160 })} />
            <Btn icon="▤" label="코드" onClick={() => addBlock({ md: '```js\nconst x = 1;\n```', w: 520, h: 160 })} />
          </Group>
        </>
      )}

      {tab === '디자인' && (
        <>
          <Group label="슬라이드 배경">
            <ColorPicker
              value={slide.canvas.bg}
              onChange={(bg) => patchSlide((s) => ({ ...s, canvas: { ...s.canvas, bg: bg ?? '#ffffff' } }))}
              title="배경색"
              accent={project.manifest?.theme?.accent}
            />
          </Group>
          <Group label="모든 슬라이드 배경">
            <ColorPicker
              value={undefined}
              onChange={(bg) =>
                setItems((list) => list.map((s) => ({ ...s, canvas: { ...s.canvas, bg: bg ?? '#ffffff' } })))
              }
              title="모든 슬라이드 배경색"
              accent={project.manifest?.theme?.accent}
            />
          </Group>
          <Group label="테마 강조색">
            <ColorPicker
              value={project.manifest?.theme?.accent}
              onChange={(accent) =>
                accent &&
                ctl.commit((p) => ({ ...p, manifest: { ...p.manifest, theme: { ...p.manifest.theme, accent } } }))
              }
              title="강조색"
              accent={project.manifest?.theme?.accent}
            />
          </Group>
          <Group label="슬라이드 크기">
            <div className="rcol">
              <Select
                value={`${slide.canvas.w}x${slide.canvas.h}`}
                onChange={(v) => {
                  const [w, h] = v.split('x').map(Number);
                  resizeCanvas(w, h);
                }}
                options={canvasSizes(slide.canvas)}
                title="슬라이드 크기"
                width={172}
              />
              <div className="rrow">
                <NumInput
                  value={Math.round(slide.canvas.w)}
                  onChange={(w) => resizeCanvas(w, slide.canvas.h)}
                  title="너비 (px)"
                  min={200}
                  max={4000}
                  step={10}
                />
                <NumInput
                  value={Math.round(slide.canvas.h)}
                  onChange={(h) => resizeCanvas(slide.canvas.w, h)}
                  title="높이 (px)"
                  min={200}
                  max={4000}
                  step={10}
                />
                <span className="rgroup__label" style={{ paddingLeft: 2 }}>
                  {ratioOf(slide.canvas.w, slide.canvas.h)}
                </span>
              </div>
            </div>
          </Group>
        </>
      )}

      {tab === '슬라이드 쇼' && (
        <>
          <Group label="시작">
            <Btn icon="▶" label="처음부터" title="F5" onClick={() => { setCurrent(0); setPresenting(true); }} />
            <Btn icon="▷" label="현재 슬라이드부터" title="Shift+F5" onClick={() => setPresenting(true)} />
          </Group>
          <Group label="발표자 노트">
            <div style={{ fontSize: 11.5, color: 'var(--ink-2)', maxWidth: 380, lineHeight: 1.6, padding: '2px 6px' }}>
              쇼 중에 <kbd>S</kbd>를 누르면 발표자 노트가 열립니다. 이동은 <kbd>→</kbd>/<kbd>Space</kbd>,
              화면 지우기는 <kbd>B</kbd>(검정)·<kbd>W</kbd>(흰색), 종료는 <kbd>Esc</kbd>.
              <br />
              노트가 있는 슬라이드: {slides.filter((s) => s.notes?.trim()).length} / {slides.length}
            </div>
          </Group>
        </>
      )}

      {tab === '도형 서식' && selected?.kind === 'shape' && (
        <>
          <Group label="도형 삽입">
            <div className="ribbon__popholder" ref={shapeAnchor}>
              <Btn
                icon="◇"
                label="도형 변경"
                pressed={shapePicker}
                onClick={() => setShapePicker((v) => !v)}
              />
              {shapePicker && (
                <Popover anchorRef={shapeAnchor} onClose={() => setShapePicker(false)} label="도형">
                <ShapeGallery
                  recent={recentShapes}
                  onPick={(preset) => {
                    patchShape(selected.id, { preset: preset.name, adjust: {} });
                    setShapePicker(false);
                    setRecentShapes((prev) =>
                      [preset, ...prev.filter((p) => p.name !== preset.name)].slice(0, 10)
                    );
                  }}
                  onClose={() => setShapePicker(false)}
                />
                </Popover>
              )}
            </div>
            <span className="ribbon__note">{shapeLabel(selected.shape)}</span>
          </Group>

          <Group label="도형 채우기">
            <ColorPicker
              title="채우기 색"
              value={selected.shape?.fill?.color ?? null}
              none="채우기 없음"
              accent={project.manifest?.theme?.accent}
              onChange={(color) =>
                patchShape(selected.id, {
                  fill: color ? { color, opacity: selected.shape?.fill?.opacity ?? 100 } : null,
                })
              }
            />
          </Group>

          <Group label="도형 윤곽선">
            <ColorPicker
              title="윤곽선 색"
              value={selected.shape?.line?.color ?? null}
              none="윤곽선 없음"
              accent={project.manifest?.theme?.accent}
              onChange={(color) =>
                color === null
                  ? patchShape(selected.id, { line: null })
                  : patchShape(selected.id, {
                  line: {
                    color,
                    width: selected.shape?.line?.width ?? 1,
                    dash: selected.shape?.line?.dash ?? 'solid',
                  },
                })
              }
            />
            <div className="ribbon__stack">
              <Select
                title="두께"
                width={70}
                value={String(selected.shape?.line?.width ?? 1)}
                options={LINE_WIDTHS}
                onChange={(value) =>
                  patchShape(selected.id, {
                    line: {
                      color: selected.shape?.line?.color ?? '#1f2937',
                      width: Number(value),
                      dash: selected.shape?.line?.dash ?? 'solid',
                    },
                  })
                }
              />
              <Select
                title="대시"
                width={90}
                value={selected.shape?.line?.dash ?? 'solid'}
                options={LINE_DASHES}
                onChange={(dash) =>
                  patchShape(selected.id, {
                    line: {
                      color: selected.shape?.line?.color ?? '#1f2937',
                      width: selected.shape?.line?.width ?? 1,
                      dash,
                    },
                  })
                }
              />
            </div>
            <Btn
              small
              icon="⌧"
              label="윤곽선 없음"
              pressed={!selected.shape?.line}
              onClick={() => patchShape(selected.id, { line: null })}
            />
          </Group>

          <Group label="회전">
            <div className="ribbon__stack">
              <div className="ribbon__row">
                <Btn small icon="↺" label="왼쪽 90°" onClick={() => rotateBy(selected, -90)} />
                <Btn small icon="↻" label="오른쪽 90°" onClick={() => rotateBy(selected, 90)} />
              </div>
              <div className="ribbon__row">
                <Btn
                  small
                  icon="↔"
                  label="좌우 대칭"
                  pressed={!!selected.shape?.flipH}
                  onClick={() => patchShape(selected.id, { flipH: !selected.shape?.flipH })}
                />
                <Btn
                  small
                  icon="↕"
                  label="상하 대칭"
                  pressed={!!selected.shape?.flipV}
                  onClick={() => patchShape(selected.id, { flipV: !selected.shape?.flipV })}
                />
              </div>
              <label className="ribbon__field">
                각도
                <input
                  type="number"
                  min="0"
                  max="359"
                  step="1"
                  value={Math.round(selected.shape?.rotation ?? 0)}
                  onChange={(e) =>
                    patchShape(selected.id, {
                      rotation: ((Number(e.target.value) % 360) + 360) % 360,
                    })
                  }
                />
              </label>
            </div>
          </Group>

          <Group label="정렬">
            <div className="ribbon__stack">
              <div className="ribbon__row">
                <Btn small icon="⬆" label="맨 앞으로" onClick={() => patchBlock(selected.id, { z: maxZ(slide) + 1 })} />
                <Btn small icon="⬇" label="맨 뒤로" onClick={() => patchBlock(selected.id, { z: minZ(slide) - 1 })} />
              </div>
              <div className="ribbon__row">
                {[
                  ['left', '⇤', '왼쪽 맞춤'],
                  ['hcenter', '⇔', '가로 가운데'],
                  ['right', '⇥', '오른쪽 맞춤'],
                  ['top', '⤒', '위 맞춤'],
                  ['vcenter', '⇕', '세로 가운데'],
                  ['bottom', '⤓', '아래 맞춤'],
                ].map(([edge, icon, title]) => (
                  <Btn key={edge} small icon={icon} label="" title={`개체 ${title}`} onClick={() => doAlign(edge)} />
                ))}
              </div>
            </div>
          </Group>

          <Group label="크기 (px)">
            <div className="ribbon__stack">
              <label className="ribbon__field">
                너비
                <input
                  type="number"
                  min="8"
                  value={Math.round(selected.w)}
                  onChange={(e) => patchBlock(selected.id, { w: Math.max(8, Number(e.target.value)) })}
                />
              </label>
              <label className="ribbon__field">
                높이
                <input
                  type="number"
                  min="8"
                  value={Math.round(selected.h)}
                  onChange={(e) => patchBlock(selected.id, { h: Math.max(8, Number(e.target.value)) })}
                />
              </label>
            </div>
          </Group>
        </>
      )}

      {tab === '표 디자인' && selected?.kind === 'table' && (
        <>
          <Group label="표 스타일 옵션">
            <div className="ribbon__stack">
              <Check
                label="머리글 행"
                checked={selected.table?.headerRow !== false}
                onChange={(v) => patchTable(selected.id, { headerRow: v })}
              />
              <Check
                label="줄무늬 행"
                checked={selected.table?.bandedRows !== false}
                onChange={(v) => patchTable(selected.id, { bandedRows: v })}
              />
              <Check
                label="첫째 열"
                checked={!!selected.table?.firstCol}
                onChange={(v) => patchTable(selected.id, { firstCol: v })}
              />
            </div>
          </Group>

          <Group label="표 스타일">
            <Select
              title="표 스타일"
              width={120}
              value={selected.table?.style ?? 'banded'}
              options={TABLE_STYLES}
              onChange={(style) => patchTable(selected.id, { style })}
            />
          </Group>

          <Group label="음영">
            <ColorPicker
              title="선택한 셀의 음영"
              value={cellFormatOf(selected, cell)?.fill ?? null}
              none="음영 없음"
              accent={project.manifest?.theme?.accent}
              onChange={(fill) => cell && editTable('format', { col: cell.col, row: cell.row, fill })}
            />
            <Btn
              small
              icon="⌧"
              label="음영 없음"
              disabled={!cell}
              onClick={() =>
                cell && editTable('format', { col: cell.col, row: cell.row, fill: '' })
              }
            />
          </Group>
        </>
      )}

      {tab === '레이아웃' && selected?.kind === 'table' && (
        <>
          <Group label="행 및 열">
            <div className="ribbon__stack">
              <div className="ribbon__row">
                <Btn
                  small
                  icon="⤒"
                  label="위에 삽입"
                  disabled={!cell}
                  onClick={() => editTable('insert', { axis: 'row', row: cell.row })}
                />
                <Btn
                  small
                  icon="⤓"
                  label="아래에 삽입"
                  disabled={!cell}
                  onClick={() => editTable('insert', { axis: 'row', row: cell.row + 1 })}
                />
              </div>
              <div className="ribbon__row">
                <Btn
                  small
                  icon="⇤"
                  label="왼쪽에 삽입"
                  disabled={!cell}
                  onClick={() => editTable('insert', { axis: 'col', col: cell.col })}
                />
                <Btn
                  small
                  icon="⇥"
                  label="오른쪽에 삽입"
                  disabled={!cell}
                  onClick={() => editTable('insert', { axis: 'col', col: cell.col + 1 })}
                />
              </div>
            </div>
          </Group>

          <Group label="삭제">
            <div className="ribbon__stack">
              <Btn
                small
                icon="⌦"
                label="행 삭제"
                disabled={!cell}
                onClick={() => editTable('delete', { axis: 'row', row: cell.row })}
              />
              <Btn
                small
                icon="⌫"
                label="열 삭제"
                disabled={!cell}
                onClick={() => editTable('delete', { axis: 'col', col: cell.col })}
              />
            </div>
          </Group>

          <Group label="병합">
            <div className="ribbon__stack">
              <Btn
                small
                icon="⊞"
                label="오른쪽과 병합"
                disabled={!cell}
                onClick={() =>
                  editTable('merge', {
                    col: cell.col,
                    row: cell.row,
                    col2: cell.col + 1,
                    row2: cell.row,
                  })
                }
              />
              <Btn
                small
                icon="⊟"
                label="아래와 병합"
                disabled={!cell}
                onClick={() =>
                  editTable('merge', {
                    col: cell.col,
                    row: cell.row,
                    col2: cell.col,
                    row2: cell.row + 1,
                  })
                }
              />
              <Btn
                small
                icon="⊠"
                label="병합 해제"
                disabled={!cell}
                onClick={() => editTable('split', { col: cell.col, row: cell.row })}
              />
            </div>
          </Group>

          <Group label="맞춤">
            <div className="ribbon__stack">
              <div className="ribbon__row">
                {[['left', '⟵'], ['center', '↔'], ['right', '⟶']].map(([value, icon]) => (
                  <Btn
                    key={value}
                    small
                    icon={icon}
                    label=""
                    title={`가로 ${value}`}
                    disabled={!cell}
                    pressed={cellFormatOf(selected, cell)?.align === value}
                    onClick={() =>
                      editTable('format', { col: cell.col, row: cell.row, align: value })
                    }
                  />
                ))}
              </div>
              <div className="ribbon__row">
                {[['top', '⤒'], ['middle', '⇕'], ['bottom', '⤓']].map(([value, icon]) => (
                  <Btn
                    key={value}
                    small
                    icon={icon}
                    label=""
                    title={`세로 ${value}`}
                    disabled={!cell}
                    pressed={cellFormatOf(selected, cell)?.valign === value}
                    onClick={() =>
                      editTable('format', { col: cell.col, row: cell.row, valign: value })
                    }
                  />
                ))}
              </div>
            </div>
          </Group>

          <Group label="셀 크기 (px)">
            <div className="ribbon__stack">
              <label className="ribbon__field">
                열 너비
                <input
                  type="number"
                  min="24"
                  disabled={!cell}
                  value={Math.round(colWidthOf(selected, cell))}
                  onChange={(e) => setColWidth(selected, cell, Number(e.target.value))}
                />
              </label>
              <label className="ribbon__field">
                행 높이
                <input
                  type="number"
                  min="16"
                  disabled={!cell}
                  value={Math.round(rowHeightOf(selected, cell))}
                  onChange={(e) => setRowHeight(selected, cell, Number(e.target.value))}
                />
              </label>
            </div>
          </Group>
        </>
      )}

      {tab === 'AI' && (
        <>
          <Group label="저장 포맷">
            <Btn icon="{ }" label={inspectorOpen ? '패널 닫기' : '패널 열기'} onClick={() => setInspectorOpen((v) => !v)} />
            <Btn icon="💾" label="지금 저장" onClick={save} disabled={!dirty} />
          </Group>
          <Group label="이 슬라이드가 AI에게 보이는 방식">
            <div style={{ fontSize: 11.5, color: 'var(--ink-2)', maxWidth: 460, lineHeight: 1.55, padding: '2px 4px' }}>
              {slide.blocks.length === 0
                ? '빈 슬라이드입니다.'
                : slide.blocks
                    .slice(0, 3)
                    .map((b) => `${positionPhrase(b, slide.canvas)} → "${firstLine(b.md, b.kind)}"`)
                    .join(' / ')}
              {slide.blocks.length > 3 ? ` … 외 ${slide.blocks.length - 3}개` : ''}
            </div>
          </Group>
        </>
      )}
    </Ribbon>
  );

  /* ----------------------------------------------------------------- render */

  return (
    <Shell
      type="deck"
      title={project.manifest?.title ?? ''}
      onTitleChange={setTitle}
      onTitleCommit={ctl.commitTitle}
      dirty={dirty}
      saving={saving}
      savedAt={savedAt}
      onSave={save}
      onHome={onHome}
      ribbon={ribbon}
      inspectorOpen={inspectorOpen}
      onToggleInspector={() => setInspectorOpen((v) => !v)}
      status={
        <>
          <span>
            슬라이드 {slideIndex + 1} / {slides.length}
          </span>
          <span>요소 {slide.blocks.length}개</span>
          {selected && (
            <span>
              선택: <code>{selected.kind}</code> · {positionPhrase(selected, slide.canvas)}
            </span>
          )}
          <span className="statusbar__spacer" />
          <button onClick={() => setZoomMode(zoomMode === 'fit' ? 'manual' : 'fit')} title="창에 맞춤 전환">
            {zoomMode === 'fit' ? '창에 맞춤' : '직접 지정'}
          </button>
          <ZoomSlider
            value={scale}
            onChange={(v) => {
              setZoomMode('manual');
              setZoom(v);
            }}
            min={0.25}
            max={2}
          />
        </>
      }
    >
      <SlideSorter
        slides={slides}
        current={slideIndex}
        onSelect={setCurrent}
        onReorder={moveSlide}
        onAdd={() => addSlide()}
        onContextMenu={(e, at) => ctx.open(e, slideMenu(at))}
        onDelete={(at) => deleteSlide(at)}
        onDuplicate={(at) => duplicateSlide(at)}
      />

      <div className="stage" ref={stageRef} style={{ display: 'flex', flexDirection: 'column' }}>
        <SlideCanvas
          slide={slide}
          scale={scale}
          selectedId={selectedId}
          editingId={editingId}
          folder={project.folder}
          onSelect={(id) => {
            setSelectedId(id);
            if (id !== editingId) setEditingId(null);
            // The cursor belongs to one table; moving away lets go of it.
            if (id !== selectedId && activeCell?.id !== id) setActiveCell(null);
            // With the brush loaded, the next object clicked takes the format.
            if (painter && id) paintOnto(id);
          }}
          onEdit={setEditingId}
          onChangeBlock={changeBlockBox}
          onChangeBlockMd={changeBlockMd}
          onAddBlock={addBlock}
          onDeleteBlock={deleteBlock}
          pendingShape={pendingShape}
          onDrawShape={(box) => {
            if (!pendingShape) return;
            const block = makeShape(pendingShape.name, { ...box, z: nextZ() });
            patchSlide((sl) => ({ ...sl, blocks: [...sl.blocks, block] }));
            setSelectedId(block.id);
            setPendingShape(null);
            setTab('도형 서식');
          }}
          activeCell={activeCell}
          onActiveCellChange={setActiveCell}
          onChangeTable={(blockId, md, table) => patchBlock(blockId, { md, table })}
          onOpenBlock={openBlock}
          onContextMenu={(e, block) => ctx.open(e, block ? blockMenu(block) : canvasMenu())}
        />
        <div className="notes">
          <div className="notes__label">발표자 노트 — AI.md에 함께 저장됩니다</div>
          <textarea
            value={slide.notes ?? ''}
            placeholder="이 슬라이드에서 말할 내용을 적으면 AI가 의도까지 읽습니다."
            onChange={(e) => patchSlide((s) => ({ ...s, notes: e.target.value }), { mergeKey: 'notes' })}
          />
        </div>
      </div>

      {inspectorOpen && <FileInspector project={project} activeIndex={slideIndex} />}

      {ctx.menu && <ContextMenu {...ctx.menu} onClose={ctx.close} />}

      {presenting && (
        <Slideshow
          slides={slides}
          start={slideIndex}
          folder={project.folder}
          onClose={() => setPresenting(false)}
        />
      )}

      {imageDialog && (
        <ImageDialog
          folder={project.folder}
          initial={imageDialog.id ? { src: imageDialog.src, alt: imageDialog.alt } : null}
          notify={notify}
          onCancel={() => setImageDialog(null)}
          onConfirm={({ src, alt }) => {
            const md = `![${alt || '이미지'}](${src})`;
            if (imageDialog.id) changeBlockMd(imageDialog.id, md);
            else addBlock({ kind: 'image', md, w: 480, h: 300, style: { fit: 'contain', radius: 6 } });
            setImageDialog(null);
          }}
        />
      )}

      {chartDialog && (
        <ChartDialog
          initial={chartDialog.spec}
          notify={notify}
          onCancel={() => setChartDialog(null)}
          onConfirm={(spec) => {
            const md = serializeChartBlock(spec);
            if (chartDialog.id) changeBlockMd(chartDialog.id, md);
            else addBlock({ kind: 'chart', md, w: 560, h: 340, style: {} });
            setChartDialog(null);
          }}
        />
      )}
    </Shell>
  );
}

const maxZ = (slide) => Math.max(0, ...slide.blocks.map((b) => b.z ?? 0));
const minZ = (slide) => Math.min(1, ...slide.blocks.map((b) => b.z ?? 0));

function firstLine(md, kind) {
  if (kind === 'chart') {
    const spec = parseChartBlock(md);
    return spec?.title || '차트';
  }
  const line = String(md ?? '')
    .split('\n')
    .map((l) => l.replace(/^#{1,6}\s*/, '').replace(/^[-+*]\s*/, '').trim())
    .find(Boolean);
  if (!line) return '(빈 요소)';
  return line.length > 22 ? `${line.slice(0, 21)}…` : line;
}
