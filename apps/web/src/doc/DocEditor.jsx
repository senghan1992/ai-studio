import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import {
  makeSection, newBlockId, PAGE_SIZES, CUSTOM_PAPER, pageDims, pageResize, resolveRunning,
  headingLevel, plainText, countWords,
  serializeChartBlock, parseChartBlock, blankTable, tableEdit,
} from '../core/index.js';

import Shell from '../components/Shell.jsx';
import FileInspector from '../components/FileInspector.jsx';
import FileMenu from '../components/FileMenu.jsx';
import FindBar from '../components/FindBar.jsx';
import ChartDialog from '../components/ChartDialog.jsx';
import TablePicker from '../components/TablePicker.jsx';
import ImageDialog from '../components/ImageDialog.jsx';
import ChartView from '../components/ChartView.jsx';
import TableView from '../components/TableView.jsx';
import MarkdownEditor from '../components/MarkdownEditor.jsx';
import {
  Ribbon, Group, Btn, Select, Check, NumInput, ColorPicker, Dialog, Field, Popover,
  ContextMenu, useContextMenu, ZoomSlider,
} from '../components/ui.jsx';
import {
  renderMarkdown, toggleWrap, toggleLinePrefix, markHtml,
} from '../lib/markdown.js';
import { textOffsetForMd } from '../lib/richText.js';
import { assetUrl, isProjectAsset } from '../api.js';
import {
  paginate, isPageBreak, PAGE_BREAK, contentHeightOf, contentWidthOf, columnWidthOf, COLUMN_GAP,
} from './paginate.js';

const TABS = ['파일', '홈', '삽입', '레이아웃', '보기', 'AI'];
/** Tabs that exist only while a table is selected. */
const CONTEXT_TABS = ['표 디자인', '표 레이아웃'];

/** Office's paragraph-alignment shortcuts. */
const PARA_ALIGN_KEYS = { l: 'left', e: 'center', r: 'right', j: 'justify' };
/** Word's Ctrl+1 / Ctrl+5 / Ctrl+2 line spacing. */
const LINE_SPACING_KEYS = { 1: 1, 2: 2, 5: 1.5 };
/** The size list Word's box offers, which Ctrl+Shift+> steps through. */
const FONT_SIZES = [8, 9, 10, 11, 12, 14, 16, 18, 20, 22, 24, 28, 32, 36, 40, 44, 48, 54, 60, 66, 72];
/** The document's own body size, and one tab stop of indent. */
const BODY_PT = 15;
const INDENT_STEP = 24;
/** Block types with no caret to put: selected, not opened for typing. */
const TEXTLESS_TYPES = new Set(['table', 'chart', 'image', 'pagebreak', 'hr']);

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
const emptyFind = { query: '', replacement: '', matchCase: false, at: null, replace: false };

export default function DocEditor({ ctl, onHome, notify, onNewProject }) {
  const { project, items: sections, setItems, setTitle, save, saving, dirty, savedAt, undo, redo, canUndo, canRedo } = ctl;

  const [sectionIndex, setSectionIndex] = useState(0);
  const [selectedId, setSelectedId] = useState(null);
  const [editingId, setEditingId] = useState(null);
  const [tab, setTab] = useState('홈');
  const [inspectorOpen, setInspectorOpen] = useState(true);
  const [zoom, setZoom] = useState(1);
  const [tableDialog, setTableDialog] = useState(null);
  const tableAnchor = useRef(null);
  // Which cell of the selected table has the cursor.
  const [activeCell, setActiveCell] = useState(null);
  const [imageDialog, setImageDialog] = useState(null);
  /** `'header'`, `'footer'`, or null. */
  const [runningDialog, setRunningDialog] = useState(null);
  const [chartDialog, setChartDialog] = useState(null);
  const [focusRequest, setFocusRequest] = useState(null);
  const [find, setFind] = useState(null);
  /** Ctrl+K: `{ text, url }` while the hyperlink dialog is open. */
  const [linkDialog, setLinkDialog] = useState(null);
  const [heights, setHeights] = useState({});
  const ctx = useContextMenu();

  const index = Math.min(sectionIndex, Math.max(0, sections.length - 1));
  const section = sections[index];

  /* ------------------------------------------------------------ mutations */

  const patchSection = useCallback(
    (updater, options) => setItems((list) => list.map((s, i) => (i === index ? updater(s) : s)), options),
    [setItems, index]
  );

  /**
   * Set or clear a header or footer.
   *
   * An all-empty one is removed rather than stored, so a document that never had
   * a header does not gain an empty object in its JSON.
   */
  const setRunning = useCallback(
    (where, value) => {
      const cleaned = {
        left: value?.left?.trim() ?? '',
        center: value?.center?.trim() ?? '',
        right: value?.right?.trim() ?? '',
      };
      const empty = !cleaned.left && !cleaned.center && !cleaned.right;
      patchSection((sec) => ({
        ...sec,
        page: { ...sec.page, [where]: empty ? null : cleaned },
      }));
    },
    [patchSection]
  );

  /**
   * The section's page at a new pixel size.
   *
   * The core decides whether that size still has a paper name, so the format
   * never ends up with a page labelled A4 that is not A4.
   */
  const resizePage = useCallback(
    (w, h) => patchSection((s) => ({ ...s, page: pageResize(s.page, w, h) })),
    [patchSection]
  );

  const patchBlock = useCallback(
    (blockId, patch, options) =>
      patchSection((s) => ({ ...s, blocks: s.blocks.map((b) => (b.id === blockId ? { ...b, ...patch } : b)) }), options),
    [patchSection]
  );

  const setBlockMd = useCallback(
    (blockId, md) => patchBlock(blockId, { md, type: classify(md) }, { mergeKey: `text:${blockId}` }),
    [patchBlock]
  );

  const patchOverride = useCallback(
    (blockId, delta) =>
      patchSection((s) => ({
        ...s,
        blocks: s.blocks.map((b) => {
          if (b.id !== blockId) return b;
          const next = { ...(b.override ?? {}), ...delta };
          if (delta.style) next.style = { ...(b.override?.style ?? {}), ...delta.style };
          return { ...b, override: next };
        }),
      })),
    [patchSection]
  );

  const clearOverride = useCallback((blockId) => patchBlock(blockId, { override: null }), [patchBlock]);

  const insertBlock = useCallback(
    (afterId, md = '', { focus = true } = {}) => {
      const id = newBlockId();
      patchSection((s) => {
        const at = s.blocks.findIndex((b) => b.id === afterId);
        const next = [...s.blocks];
        next.splice(at < 0 ? next.length : at + 1, 0, { id, md, type: classify(md), override: null });
        return { ...s, blocks: next };
      });
      if (focus) {
        setSelectedId(id);
        // Blocks without a caret (tables, images, page breaks, rules) are only
        // selected — they are edited through their own surfaces.
        if (!TEXTLESS_TYPES.has(classify(md))) {
          setEditingId(id);
          // The caret lands past the inserted text, ready to type. The offset
          // is a rendered-text offset — the same currency the editor's caret
          // maths speak — not a markdown offset.
          setFocusRequest({ id, caret: textOffsetForMd(md, String(md).length) });
        }
      }
      return id;
    },
    [patchSection]
  );

  /**
   * Insert a table at the cursor.
   *
   * The markdown carries the cells and the spec carries the layout, so a table
   * inserted here and one imported from Word are the same kind of thing.
   */
  const insertTable = useCallback(
    (columns, rows) => {
      const { md, table } = blankTable(columns, rows);
      const id = newBlockId();
      patchSection((s) => {
        const at = s.blocks.findIndex((b) => b.id === selectedId);
        const next = [...s.blocks];
        next.splice(at < 0 ? next.length : at + 1, 0, {
          id,
          md,
          type: 'table',
          override: null,
          table,
        });
        return { ...s, blocks: next };
      });
      setSelectedId(id);
      setTableDialog(null);
      return id;
    },
    [patchSection, selectedId]
  );

  const splitBlock = useCallback(
    (blockId, caret) => {
      const block = section?.blocks.find((b) => b.id === blockId);
      if (!block) return;
      const md = String(block.md ?? '');
      const head = md.slice(0, caret);
      const tail = md.slice(caret);
      const newId = newBlockId();
      patchSection((s) => {
        const at = s.blocks.findIndex((b) => b.id === blockId);
        if (at < 0) return s;
        const next = [...s.blocks];
        next[at] = { ...block, md: head, type: classify(head) };
        next.splice(at + 1, 0, { id: newId, md: tail, type: classify(tail), override: block.override ?? null });
        return { ...s, blocks: next };
      });
      setSelectedId(newId);
      // The caret moves into the new block — unless the split produced a block
      // that has no caret to put there (a table drawn from pipes, say).
      if (!TEXTLESS_TYPES.has(classify(tail))) {
        setEditingId(newId);
        setFocusRequest({ id: newId, caret: 0 });
      }
    },
    [patchSection, section]
  );

  const mergeBackward = useCallback(
    (blockId, caretInMd = 0) => {
      let target = null;
      let caret = 0;
      patchSection((s) => {
        const at = s.blocks.findIndex((b) => b.id === blockId);
        if (at <= 0) return s;
        const prev = s.blocks[at - 1];
        const block = s.blocks[at];
        const md = String(block.md ?? '');
        // Backspace at the very start of a heading lands after its `# `, so
        // joining drops the invisible marker — the text merges as plain words.
        const [kept, dropped] =
          caretInMd > 0 ? [md.slice(caretInMd), md.slice(0, caretInMd)] : [md, ''];
        target = prev.id;
        const joined = `${prev.md ?? ''}${dropped}`;
        caret = textOffsetForMd(joined, joined.length);
        const next = [...s.blocks];
        next[at - 1] = { ...prev, md: `${prev.md ?? ''}${kept}` };
        next.splice(at, 1);
        return { ...s, blocks: next };
      });
      if (target) {
        setSelectedId(target);
        setEditingId(target);
        setFocusRequest({ id: target, caret });
      }
    },
    [patchSection]
  );

  const deleteBlock = useCallback(
    (blockId) => {
      patchSection((s) => (s.blocks.length <= 1 ? s : { ...s, blocks: s.blocks.filter((b) => b.id !== blockId) }));
      setSelectedId(null);
      setEditingId(null);
    },
    [patchSection]
  );

  const duplicateBlock = useCallback(
    (blockId) => {
      const block = section?.blocks.find((b) => b.id === blockId);
      if (!block) return;
      insertBlock(blockId, block.md, { focus: false });
    },
    [section, insertBlock]
  );

  const moveBlock = useCallback(
    (blockId, delta) =>
      patchSection((s) => {
        const at = s.blocks.findIndex((b) => b.id === blockId);
        const to = at + delta;
        if (at < 0 || to < 0 || to >= s.blocks.length) return s;
        const next = [...s.blocks];
        const [moved] = next.splice(at, 1);
        next.splice(to, 0, moved);
        return { ...s, blocks: next };
      }),
    [patchSection]
  );

  const applyStyle = useCallback(
    (prefix) => {
      if (!selectedId) return;
      const block = section?.blocks.find((b) => b.id === selectedId);
      if (!block) return;
      const stripped = String(block.md ?? '').replace(/^#{1,6}\s+/, '');
      setBlockMd(selectedId, prefix ? `${prefix}${stripped}` : stripped);
    },
    [selectedId, section, setBlockMd]
  );

  const transformMd = useCallback(
    (transform) => {
      if (!selectedId) return;
      const block = section?.blocks.find((b) => b.id === selectedId);
      if (!block) return;
      const md = String(block.md ?? '');
      setBlockMd(selectedId, transform(md, 0, md.length).value);
    },
    [selectedId, section, setBlockMd]
  );

  /* -------------------------------------------------------- format painter */

  /**
   * 서식 복사 — carry one paragraph's formatting to another.
   *
   * The paragraph's whole `override` is copied, so the brush replaces the
   * target's formatting instead of adding to it, the way Word's does. The text
   * itself is never touched.
   */
  const [painter, setPainter] = useState(null);

  const pickUpFormat = useCallback(() => {
    if (painter) {
      setPainter(null);
      return;
    }
    const block = (section?.blocks ?? []).find((b) => b.id === selectedId);
    if (!block) return;
    setPainter(block.override ? JSON.parse(JSON.stringify(block.override)) : null);
    notify('문단 서식을 집었습니다 — 붙일 문단을 누르세요');
  }, [painter, section, selectedId, notify]);

  const paintOnto = useCallback(
    (blockId) => {
      patchBlock(blockId, { override: painter ? JSON.parse(JSON.stringify(painter)) : null });
      setPainter(null);
      notify('문단 서식을 붙였습니다');
    },
    [painter, patchBlock, notify]
  );

  /* -------------------------------------------------------------- sections */

  const addSection = () => {
    setItems((list) => [...list, makeSection({ name: `섹션 ${sections.length + 1}` })]);
    setSectionIndex(sections.length);
  };

  const deleteSection = () => {
    if (sections.length <= 1) {
      notify('마지막 섹션은 삭제할 수 없습니다');
      return;
    }
    setItems((list) => list.filter((_, i) => i !== index));
    setSectionIndex(Math.max(0, index - 1));
  };

  /* ------------------------------------------------------------------ find */

  const findHits = useMemo(() => {
    if (!find?.query || !section) return null;
    const needle = find.matchCase ? find.query : find.query.toLowerCase();
    const hits = [];
    section.blocks.forEach((block) => {
      const text = find.matchCase ? String(block.md ?? '') : String(block.md ?? '').toLowerCase();
      let at = text.indexOf(needle);
      while (at !== -1) {
        hits.push({ id: block.id, at });
        at = text.indexOf(needle, at + needle.length);
      }
    });
    return hits;
  }, [section, find?.query, find?.matchCase]);

  /*
   * Land on the first match as soon as there is one.
   *
   * Office moves the selection while you type; without this the counter reads
   * "– / 12" until you press Enter, which looks broken.
   */
  useEffect(() => {
    if (!find?.query || find.at !== null || !findHits?.length) return;
    const hit = findHits[0];
    setFind((f) => (f && f.at === null ? { ...f, at: 0 } : f));
    setSelectedId(hit.id);
    document.getElementById(`block-${hit.id}`)?.scrollIntoView({ block: 'center' });
  }, [find?.query, find?.at, findHits]);

  const stepFind = useCallback(
    (delta) => {
      if (!findHits?.length) return;
      const next = ((find.at ?? -1) + delta + findHits.length) % findHits.length;
      setFind((f) => ({ ...f, at: next }));
      setSelectedId(findHits[next].id);
      document.getElementById(`block-${findHits[next].id}`)?.scrollIntoView({ block: 'center' });
    },
    [findHits, find]
  );

  const replaceAll = useCallback(() => {
    if (!find?.query) return;
    const pattern = new RegExp(escapeRe(find.query), find.matchCase ? 'g' : 'gi');
    let count = 0;
    patchSection((s) => ({
      ...s,
      blocks: s.blocks.map((b) => {
        const next = String(b.md ?? '').replace(pattern, () => {
          count++;
          return find.replacement ?? '';
        });
        return next === b.md ? b : { ...b, md: next, type: classify(next) };
      }),
    }));
    notify(`${count}곳 바꿨습니다`);
    setFind((f) => ({ ...f, at: null }));
  }, [find, patchSection, notify]);

  const replaceCurrent = useCallback(() => {
    const hit = findHits?.[find?.at ?? 0];
    if (!hit) return;
    const block = section.blocks.find((b) => b.id === hit.id);
    if (!block) return;
    const md = String(block.md ?? '');
    const next = md.slice(0, hit.at) + (find.replacement ?? '') + md.slice(hit.at + find.query.length);
    setBlockMd(hit.id, next);
  }, [findHits, find, section, setBlockMd]);

  /* --------------------------------------------------- formatting commands */

  /** The selected paragraph, read fresh — used by handlers defined up here. */
  const blockOf = useCallback(
    (id) => (section?.blocks ?? []).find((b) => b.id === id) ?? null,
    [section]
  );

  /** Toggle one boolean run style on the selected paragraph. */
  const toggleRunStyle = useCallback(
    (key) => {
      if (!selectedId) return;
      const on = blockOf(selectedId)?.override?.style?.[key];
      patchOverride(selectedId, { style: { [key]: on ? null : true } });
    },
    [selectedId, blockOf, patchOverride]
  );

  /**
   * Grow or shrink the paragraph one step through Word's own size list.
   *
   * Ctrl+Shift+> walks the list rather than adding a fixed number of points,
   * which is why 11pt goes to 12 and 48 goes to 72.
   */
  const stepFontSize = useCallback(
    (direction) => {
      if (!selectedId) return;
      const current = blockOf(selectedId)?.override?.style?.fontSize ?? BODY_PT;
      const at = FONT_SIZES.findIndex((n) => n >= current);
      const from = at < 0 ? FONT_SIZES.length - 1 : at;
      const next = FONT_SIZES[Math.min(FONT_SIZES.length - 1, Math.max(0, from + direction))];
      patchOverride(selectedId, { style: { fontSize: next } });
    },
    [selectedId, blockOf, patchOverride]
  );

  /** Change the paragraph's left indent by one Word tab stop. */
  const stepIndent = useCallback(
    (direction) => {
      if (!selectedId) return;
      const current = blockOf(selectedId)?.override?.indent ?? 0;
      patchOverride(selectedId, { indent: Math.max(0, current + direction * INDENT_STEP) || null });
    },
    [selectedId, blockOf, patchOverride]
  );

  /**
   * Move the caret into the paragraph before or after this one.
   *
   * Holding the down arrow to read through a document is the most ordinary
   * thing there is, and stopping dead at every paragraph boundary is the
   * clearest sign that a block editor is not a word processor. A block with no
   * text to put a caret in — a table, an image, a page break — is selected
   * instead of opened.
   */
  const stepParagraph = useCallback(
    (fromId, direction) => {
      const blocks = section?.blocks ?? [];
      const at = blocks.findIndex((b) => b.id === fromId);
      const target = blocks[at + direction];
      if (at < 0 || !target) return false;
      setSelectedId(target.id);
      if (TEXTLESS_TYPES.has(target.type) || isPageBreak(target)) {
        setEditingId(null);
        document.getElementById(`block-${target.id}`)?.scrollIntoView({ block: 'nearest' });
        return true;
      }
      setEditingId(target.id);
      setFocusRequest({
        id: target.id,
        caret: direction > 0 ? 0 : textOffsetForMd(target.md ?? '', String(target.md ?? '').length),
      });
      return true;
    },
    [section]
  );

  /* ------------------------------------------------------------------ keys */

  useEffect(() => {
    const onKey = (e) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        setFind((f) => f ?? { ...emptyFind });
        return;
      }
      if (mod && e.key.toLowerCase() === 'h') {
        e.preventDefault();
        setFind((f) => ({ ...(f ?? emptyFind), replace: true }));
        return;
      }
      if (mod && e.key.toLowerCase() === 'p') {
        e.preventDefault();
        window.print();
        return;
      }
      if (mod && e.key === 'Enter') {
        e.preventDefault();
        insertBlock(selectedId, PAGE_BREAK, { focus: false });
        notify('페이지 나누기를 넣었습니다');
        return;
      }
      if (e.key === 'Escape' && find) {
        setFind(null);
        return;
      }

      /*
       * Word's formatting shortcuts.
       *
       * They have to work with the caret in the paragraph — that is the only
       * time anyone presses them — so the paragraph's own textarea is allowed
       * through while the find box and the title field are not.
       */
      const inParagraph = !!e.target?.classList?.contains?.('docblock__editor');
      const tag = e.target?.tagName;
      if ((tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA') && !inParagraph) return;
      if (!mod) return;

      // Ctrl+Home / Ctrl+End — the top and the bottom of the document.
      if (e.key === 'Home' || e.key === 'End') {
        const blocks = section?.blocks ?? [];
        const target = e.key === 'Home' ? blocks[0] : blocks[blocks.length - 1];
        if (!target) return;
        e.preventDefault();
        setSelectedId(target.id);
        setEditingId(null);
        document.getElementById(`block-${target.id}`)?.scrollIntoView({ block: 'center' });
        return;
      }

      if (!selectedId) return;
      const key = e.key.toLowerCase();

      // Ctrl+U — 밑줄. The .docx writer already emits <w:u/> for this flag, so
      // it survives the trip to Word; markdown has no underline, which is why it
      // lives in the paragraph's meta.json rather than in the text.
      if (key === 'u' && !e.shiftKey && !e.altKey) {
        e.preventDefault();
        toggleRunStyle('underline');
        return;
      }
      // Ctrl+Alt+1/2/3 — 제목 1·2·3, Ctrl+Shift+N — 본문. Word's own keys.
      if (e.altKey && '123456'.includes(e.key)) {
        e.preventDefault();
        applyStyle(`${'#'.repeat(Number(e.key))} `);
        return;
      }
      if (e.shiftKey && key === 'n') {
        e.preventDefault();
        applyStyle(null);
        return;
      }
      // Ctrl+L / E / R / J — 문단 맞춤.
      if (!e.shiftKey && !e.altKey && PARA_ALIGN_KEYS[key]) {
        e.preventDefault();
        patchOverride(selectedId, { align: PARA_ALIGN_KEYS[key] });
        return;
      }
      // Ctrl+Shift+> / < — 글자 크기 한 단계.
      if (e.shiftKey && (e.key === '>' || e.key === '<' || e.key === '.' || e.key === ',')) {
        e.preventDefault();
        stepFontSize(e.key === '>' || e.key === '.' ? 1 : -1);
        return;
      }
      // Ctrl+1 / 2 / 5 — 줄 간격 1.0 · 2.0 · 1.5, exactly as in Word.
      if (!e.altKey && !e.shiftKey && LINE_SPACING_KEYS[e.key]) {
        e.preventDefault();
        patchOverride(selectedId, { style: { lineHeight: LINE_SPACING_KEYS[e.key] } });
        return;
      }
      // Ctrl+K — 하이퍼링크.
      if (key === 'k' && !e.shiftKey && !e.altKey) {
        e.preventDefault();
        setLinkDialog({ text: '', url: 'https://' });
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [find, insertBlock, selectedId, notify, section, patchOverride, applyStyle, stepFontSize, toggleRunStyle]);

  /* ------------------------------------------------------------ pagination */

  const page = section?.page ?? { size: 'A4', margin: { top: 72, right: 72, bottom: 72, left: 72 } };
  // The core resolves the size: a landscape or hand-typed page has no paper name
  // to look up, and an imported document is full of both.
  const pageSize = useMemo(() => pageDims(page), [page]);
  const columns = Math.max(1, Math.min(4, Math.round(page.columns ?? 1)));
  const contentHeight = contentHeightOf(pageSize, page.margin);
  const contentWidth = contentWidthOf(pageSize, page.margin);
  // Blocks are measured at the width they will be drawn at, and a page holds one
  // column-height per column.
  const columnWidth = columnWidthOf(contentWidth, columns);
  const pageCapacity = contentHeight * columns;

  const pages = useMemo(
    () => paginate(section?.blocks ?? [], heights, pageCapacity),
    [section?.blocks, heights, pageCapacity]
  );

  /**
   * Measure every block at the page's content width, then let `paginate` place
   * them. The hidden layer is what makes the split honest: a paragraph is moved to
   * the next page only when its real rendered height does not fit.
   */
  const measureRefs = useRef(new Map());
  useLayoutEffect(() => {
    const next = {};
    let changed = false;
    for (const [id, node] of measureRefs.current) {
      if (!node) continue;
      const h = node.offsetHeight;
      next[id] = h;
      if (Math.abs((heights[id] ?? -1) - h) > 0.5) changed = true;
    }
    if (changed || Object.keys(next).length !== Object.keys(heights).length) setHeights(next);
  }, [section?.blocks, contentWidth, zoom, heights]);

  const stats = useMemo(() => {
    const text = sections.flatMap((s) => s.blocks.map((b) => plainText(b.md))).join('\n');
    return { words: countWords(text), chars: text.replace(/\s/g, '').length };
  }, [sections]);

  const outline = useMemo(
    () =>
      (section?.blocks ?? [])
        .map((b) => ({ id: b.id, level: headingLevel(b.md), text: plainText(b.md) }))
        .filter((o) => o.level > 0 && o.text),
    [section]
  );

  const selected = section?.blocks.find((b) => b.id === selectedId) ?? null;
  /** The cursor, but only while it belongs to the selected table. */
  const cell = activeCell?.id === selected?.id ? activeCell : null;

  useEffect(() => {
    if (CONTEXT_TABS.includes(tab) && selected?.type !== 'table') setTab('홈');
  }, [tab, selected?.type]);

  const patchTable = (blockId, partial) => {
    const block = section?.blocks.find((b) => b.id === blockId);
    if (!block) return;
    patchBlock(blockId, { table: { ...(block.table ?? {}), ...partial } });
  };

  /**
   * One structural edit to the selected table.
   *
   * The markdown and the layout change together — inserting a column has to
   * shift the merges — so the Rust core returns both and they are patched as one.
   */
  const editTable = (op, args = {}) => {
    const block = section?.blocks.find((b) => b.id === selectedId);
    if (!block || block.type !== 'table') return;
    const next = tableEdit(block.md, block.table, op, args);
    patchBlock(block.id, { md: next.md, table: next.table });
  };

  const openBlock = useCallback((block) => {
    const chart = parseChartBlock(block.md);
    if (chart) {
      setChartDialog({ id: block.id, spec: chart });
      return true;
    }
    const image = String(block.md ?? '').match(/^!\[([^\]]*)\]\(([^)\s]+)/);
    if (image) {
      setImageDialog({ id: block.id, alt: image[1], src: image[2] });
      return true;
    }
    return false;
  }, []);

  if (!section) return <div className="center-note">문서를 불러오는 중…</div>;

  /* -------------------------------------------------------- context menus */

  const blockMenu = (block) => [
    { label: '위에 문단 추가', onClick: () => insertBlock(prevIdOf(section, block.id), '') },
    { label: '아래에 문단 추가', onClick: () => insertBlock(block.id, '') },
    { label: '복제', onClick: () => duplicateBlock(block.id) },
    '-',
    { label: '위로 이동', onClick: () => moveBlock(block.id, -1) },
    { label: '아래로 이동', onClick: () => moveBlock(block.id, 1) },
    '-',
    ...(parseChartBlock(block.md)
      ? [{ label: '차트 편집', onClick: () => openBlock(block) }]
      : /^!\[/.test(String(block.md ?? ''))
      ? [{ label: '이미지 편집', onClick: () => openBlock(block) }]
      : []),
    { label: '아래에 페이지 나누기', onClick: () => insertBlock(block.id, PAGE_BREAK, { focus: false }) },
    ...(painter !== null ? [{ label: '집어 둔 서식 붙이기', onClick: () => paintOnto(block.id) }] : []),
    ...(block.override ? [{ label: '서식 지우기', onClick: () => clearOverride(block.id) }] : []),
    '-',
    { label: '삭제', danger: true, disabled: section.blocks.length <= 1, onClick: () => deleteBlock(block.id) },
  ];

  /* ---------------------------------------------------------------- ribbon */

  const ribbon = (
    <Ribbon
      tabs={TABS}
      active={tab}
      onTab={setTab}
      contextual={selected?.type === 'table' ? CONTEXT_TABS : []}
      contextLabel={selected?.type === 'table' ? '표 도구' : null}
    >
      {tab === '파일' && (
        <FileMenu
          project={project}
          dirty={dirty}
          onSave={save}
          onHome={onHome}
          onNewProject={onNewProject}
          onPrint={() => window.print()}
          notify={notify}
        />
      )}

      {tab === '홈' && (
        <>
          <Group label="되돌리기">
            <Btn icon="↶" label="실행 취소" onClick={undo} disabled={!canUndo} title="Ctrl+Z" />
            <Btn icon="↷" label="다시 실행" onClick={redo} disabled={!canRedo} title="Ctrl+Shift+Z" />
          </Group>

          <Group label="스타일">
            <div className="rcol">
              <Select
                value={styleOf(selected?.md)}
                onChange={(v) => applyStyle(v === 'body' ? null : `${'#'.repeat(Number(v))} `)}
                options={[
                  { value: 'body', label: '본문' },
                  { value: '1', label: '제목 1' },
                  { value: '2', label: '제목 2' },
                  { value: '3', label: '제목 3' },
                  { value: '4', label: '제목 4' },
                  { value: '5', label: '제목 5' },
                  { value: '6', label: '제목 6' },
                ]}
                title="문단 스타일 (Ctrl+Alt+1~3 · 본문 Ctrl+Shift+N)"
                width={104}
              />
              <div className="rrow">
                <Btn small label="H1" title="제목 1" disabled={!selected} onClick={() => applyStyle('# ')} />
                <Btn small label="H2" title="제목 2" disabled={!selected} onClick={() => applyStyle('## ')} />
                <Btn small label="H3" title="제목 3" disabled={!selected} onClick={() => applyStyle('### ')} />
              </div>
            </div>
          </Group>

          <Group label="글꼴">
            <div className="rcol">
              <div className="rrow">
                <Btn small icon="B" label="" title="굵게 (Ctrl+B)" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleWrap(md, a, b, '**'))} />
                <Btn small icon="I" label="" title="기울임 (Ctrl+I)" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleWrap(md, a, b, '*'))} />
                {/* 밑줄은 마크다운에 없어 문단의 meta.json에 기록되고, 내보낼 때
                    Word의 <w:u/>가 됩니다 — Ctrl+U가 없다는 것이 Word 사용자에게
                    가장 먼저 걸리는 부분이라 서식 항목으로 지원합니다. */}
                <Btn
                  small
                  icon="U"
                  label=""
                  title="밑줄 (Ctrl+U)"
                  disabled={!selected}
                  pressed={!!selected?.override?.style?.underline}
                  onClick={() => toggleRunStyle('underline')}
                />
                <Btn small icon="S" label="" title="취소선 (~~)" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleWrap(md, a, b, '~~'))} />
                <Btn small icon="`" label="" title="인라인 코드" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleWrap(md, a, b, '`'))} />
              </div>
              <div className="rrow">
                <Select
                  value={String(selected?.override?.style?.fontSize ?? BODY_PT)}
                  onChange={(v) => selected && patchOverride(selected.id, { style: { fontSize: Number(v) } })}
                  options={[
                    ...(FONT_SIZES.includes(selected?.override?.style?.fontSize ?? BODY_PT)
                      ? []
                      : [{ value: String(selected?.override?.style?.fontSize ?? BODY_PT), label: String(selected?.override?.style?.fontSize ?? BODY_PT) }]),
                    ...FONT_SIZES.map((n) => ({ value: String(n), label: String(n) })),
                  ]}
                  title="글자 크기"
                  width={62}
                />
                <Btn small label="A+" title="크게 (Ctrl+Shift+>)" disabled={!selected} onClick={() => stepFontSize(1)} />
                <Btn small label="A−" title="작게 (Ctrl+Shift+<)" disabled={!selected} onClick={() => stepFontSize(-1)} />
                <ColorPicker
                  value={selected?.override?.style?.color}
                  onChange={(c) => selected && patchOverride(selected.id, { style: { color: c } })}
                  title="글자 색"
                  none="자동"
                  accent={project.manifest?.theme?.accent}
                  disabled={!selected}
                />
                <ColorPicker
                  value={selected?.override?.style?.bg}
                  onChange={(c) => selected && patchOverride(selected.id, { style: { bg: c } })}
                  title="형광펜 (문단 강조색)"
                  none="강조 없음"
                  accent={project.manifest?.theme?.accent}
                  disabled={!selected}
                />
              </div>
            </div>
          </Group>

          <Group label="문단">
            <div className="rcol">
              <div className="rrow">
                <Btn small icon="•" label="" title="글머리 기호" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleLinePrefix(md, a, b, '- '))} />
                <Btn small icon="1." label="" title="번호 매기기" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleLinePrefix(md, a, b, '1. '))} />
                <Btn small icon="❝" label="" title="인용" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleLinePrefix(md, a, b, '> '))} />
              </div>
              <div className="rrow">
                {[
                  ['left', '⬅', '왼쪽'],
                  ['center', '↔', '가운데'],
                  ['right', '➡', '오른쪽'],
                  ['justify', '☰', '양쪽'],
                ].map(([value, icon, title]) => (
                  <Btn
                    key={value}
                    small
                    icon={icon}
                    label=""
                    title={`${title} 정렬 (Ctrl+${{ left: 'L', center: 'E', right: 'R', justify: 'J' }[value]})`}
                    disabled={!selected}
                    pressed={(selected?.override?.align ?? 'left') === value}
                    onClick={() => patchOverride(selected.id, { align: value })}
                  />
                ))}
                <Btn small icon="⇥" label="" title="들여쓰기 늘리기 (Tab)" disabled={!selected} onClick={() => stepIndent(1)} />
                <Btn small icon="⇤" label="" title="들여쓰기 줄이기 (Shift+Tab)" disabled={!selected} onClick={() => stepIndent(-1)} />
              </div>
              <div className="rrow">
                {/* 줄 간격 · 문단 간격 — Word의 문단 그룹에 있는 그대로.
                    둘 다 .meta.json에 기록되고 내보낼 때 w:spacing이 됩니다. */}
                <Select
                  value={String(selected?.override?.style?.lineHeight ?? 'auto')}
                  onChange={(v) =>
                    selected && patchOverride(selected.id, { style: { lineHeight: v === 'auto' ? null : Number(v) } })
                  }
                  options={[
                    { value: 'auto', label: '줄 간격 기본' },
                    { value: '1', label: '1.0' },
                    { value: '1.15', label: '1.15' },
                    { value: '1.5', label: '1.5' },
                    { value: '2', label: '2.0' },
                    { value: '2.5', label: '2.5' },
                    { value: '3', label: '3.0' },
                  ]}
                  title="줄 간격 (Ctrl+1 · Ctrl+5 · Ctrl+2)"
                  width={104}
                />
                <NumInput
                  value={selected?.override?.spacing?.before ?? 0}
                  onChange={(v) =>
                    selected &&
                    patchOverride(selected.id, {
                      spacing: { ...(selected.override?.spacing ?? {}), before: v || null },
                    })
                  }
                  title="문단 위 간격 (px)"
                  min={0}
                  max={200}
                  step={4}
                />
                <NumInput
                  value={selected?.override?.spacing?.after ?? 0}
                  onChange={(v) =>
                    selected &&
                    patchOverride(selected.id, {
                      spacing: { ...(selected.override?.spacing ?? {}), after: v || null },
                    })
                  }
                  title="문단 아래 간격 (px)"
                  min={0}
                  max={200}
                  step={4}
                />
              </div>
            </div>
          </Group>

          <Group label="편집">
            <Btn icon="🔗" label="하이퍼링크" title="Ctrl+K" onClick={() => setLinkDialog({ text: '', url: 'https://' })} />
            <Btn icon="🔍" label="찾기" title="Ctrl+F" onClick={() => setFind((f) => f ?? { ...emptyFind })} />
            <Btn icon="⇄" label="바꾸기" title="Ctrl+H" onClick={() => setFind((f) => ({ ...(f ?? emptyFind), replace: true }))} />
            <Btn
              icon="🖌"
              label={painter !== null ? '집은 상태' : '서식 복사'}
              title="이 문단의 서식만 다른 문단에 옮깁니다 (글은 그대로)"
              pressed={painter !== null}
              disabled={!selected && painter === null}
              onClick={pickUpFormat}
            />
            <Btn
              icon="⌫"
              label="서식 지우기"
              title="이 문단의 JSON 서식 항목을 제거합니다"
              disabled={!selected?.override}
              onClick={() => clearOverride(selected.id)}
            />
          </Group>
        </>
      )}

      {tab === '삽입' && (
        <>
          <Group label="블록">
            <Btn icon="¶" label="문단" onClick={() => insertBlock(selectedId, '')} />
            <Btn icon="H" label="제목" onClick={() => insertBlock(selectedId, '## 새 제목')} />
            <Btn icon="≔" label="목록" onClick={() => insertBlock(selectedId, '- 항목 1\n- 항목 2')} />
            <Btn icon="❝" label="인용" onClick={() => insertBlock(selectedId, '> 인용문')} />
            <Btn icon="—" label="구분선" onClick={() => insertBlock(selectedId, '---')} />
          </Group>
          <Group label="그림 · 차트">
            <Btn icon="🖼" label="이미지" onClick={() => setImageDialog({ src: '', alt: '' })} />
            <Btn icon="📊" label="차트" onClick={() => setChartDialog({ spec: null })} />
            <div className="ribbon__popholder" ref={tableAnchor}>
              <Btn
                icon="▦"
                label="표"
                pressed={!!tableDialog}
                onClick={() => setTableDialog(tableDialog ? null : {})}
              />
              {tableDialog && (
                <Popover anchorRef={tableAnchor} onClose={() => setTableDialog(null)} label="표 삽입">
                  <TablePicker
                    onPick={(columns, rows) => insertTable(columns, rows)}
                    onClose={() => setTableDialog(null)}
                  />
                </Popover>
              )}
            </div>
            <Btn icon="▤" label="코드" onClick={() => insertBlock(selectedId, '```js\nconst x = 1;\n```')} />
          </Group>
          <Group label="페이지">
            <Btn
              icon="⤓"
              label="페이지 나누기"
              title="Ctrl+Enter"
              onClick={() => insertBlock(selectedId, PAGE_BREAK, { focus: false })}
            />
          </Group>
          <Group label="머리글 및 바닥글">
            <Btn icon="▔" label="머리글" pressed={runningDialog === 'header'} onClick={() => setRunningDialog('header')} />
            <Btn icon="▁" label="바닥글" pressed={runningDialog === 'footer'} onClick={() => setRunningDialog('footer')} />
            <Btn
              icon="#"
              label="페이지 번호"
              title="바닥글 가운데에 페이지 번호를 넣습니다"
              onClick={() => setRunning('footer', { ...(page.footer ?? {}), center: '{PAGE}' })}
            />
          </Group>
          <Group label="섹션">
            <Btn icon="🞤" label="섹션 추가" onClick={addSection} />
            <Btn icon="🗑" label="섹션 삭제" onClick={deleteSection} disabled={sections.length <= 1} />
          </Group>
        </>
      )}

      {tab === '레이아웃' && (
        <>
          <Group label="여백 (px)">
            <div className="rrow">
              {['top', 'right', 'bottom', 'left'].map((side) => (
                <NumInput
                  key={side}
                  value={page.margin?.[side] ?? 72}
                  onChange={(v) =>
                    patchSection((s) => ({ ...s, page: { ...s.page, margin: { ...s.page.margin, [side]: v } } }))
                  }
                  title={`${side} 여백`}
                  min={0}
                  max={300}
                  step={12}
                />
              ))}
            </div>
          </Group>
          <Group label="용지 방향">
            <Btn
              icon="▯"
              label="세로"
              pressed={!pageSize.landscape}
              onClick={() => pageSize.landscape && resizePage(pageSize.h, pageSize.w)}
            />
            <Btn
              icon="▭"
              label="가로"
              pressed={pageSize.landscape}
              onClick={() => !pageSize.landscape && resizePage(pageSize.h, pageSize.w)}
            />
          </Group>
          <Group label="용지 크기">
            <Select
              value={PAGE_SIZES[page.size] ? page.size : CUSTOM_PAPER.label}
              onChange={(name) => {
                const paper = PAGE_SIZES[name];
                if (!paper) return;
                // Choosing A4 on a landscape page gives landscape A4, which is
                // what Word does: the size list and the orientation are separate
                // controls and neither resets the other.
                resizePage(pageSize.landscape ? paper.h : paper.w, pageSize.landscape ? paper.w : paper.h);
              }}
              options={[
                ...Object.keys(PAGE_SIZES).map((k) => ({ value: k, label: k })),
                ...(PAGE_SIZES[page.size] ? [] : [{ value: CUSTOM_PAPER.label, label: CUSTOM_PAPER.label }]),
              ]}
              title="용지 크기"
              width={110}
            />
          </Group>
          <Group label="단">
            <Select
              value={String(columns)}
              onChange={(v) => patchSection((sec) => ({ ...sec, page: { ...sec.page, columns: Number(v) } }))}
              options={[
                { value: '1', label: '한 단' },
                { value: '2', label: '두 단' },
                { value: '3', label: '세 단' },
              ]}
              title="단"
              width={92}
            />
          </Group>
          <Group label="크기 (px)">
            <div className="rrow">
              <NumInput
                value={Math.round(pageSize.w)}
                onChange={(v) => resizePage(v, pageSize.h)}
                title="너비"
                min={200}
                max={4000}
                step={10}
              />
              <NumInput
                value={Math.round(pageSize.h)}
                onChange={(v) => resizePage(pageSize.w, v)}
                title="높이"
                min={200}
                max={4000}
                step={10}
              />
            </div>
          </Group>
          <Group label="섹션 이름">
            <input
              className="rinput"
              style={{ width: 160 }}
              value={section.name}
              onChange={(e) => patchSection((s) => ({ ...s, name: e.target.value }), { mergeKey: 'secname' })}
              aria-label="섹션 이름"
            />
          </Group>
        </>
      )}

      {tab === '보기' && (
        <>
          <Group label="인쇄">
            <Btn icon="🖨" label="인쇄 / PDF" title="Ctrl+P" onClick={() => window.print()} />
          </Group>
          <Group label="확대/축소">
            <Select
              value={String(zoom)}
              onChange={(v) => setZoom(Number(v))}
              options={[
                { value: '0.5', label: '50%' },
                { value: '0.75', label: '75%' },
                { value: '1', label: '100%' },
                { value: '1.25', label: '125%' },
                { value: '1.5', label: '150%' },
              ]}
              title="확대/축소"
              width={100}
            />
          </Group>
          <Group label="페이지">
            <div style={{ fontSize: 11.5, color: 'var(--ink-2)', padding: '2px 6px', lineHeight: 1.6 }}>
              {pages.length}페이지 · 페이지당 {Math.round(contentHeight)}px
              <br />
              측정된 블록 {Object.keys(heights).length} / {section.blocks.length}
            </div>
          </Group>
        </>
      )}

      {tab === '표 디자인' && selected?.type === 'table' && (
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
              onClick={() => cell && editTable('format', { col: cell.col, row: cell.row, fill: '' })}
            />
          </Group>
        </>
      )}

      {tab === '표 레이아웃' && selected?.type === 'table' && (
        <>
          <Group label="행 및 열">
            <div className="ribbon__stack">
              <div className="ribbon__row">
                <Btn small icon="⤒" label="위에 삽입" disabled={!cell}
                  onClick={() => editTable('insert', { axis: 'row', row: cell.row })} />
                <Btn small icon="⤓" label="아래에 삽입" disabled={!cell}
                  onClick={() => editTable('insert', { axis: 'row', row: cell.row + 1 })} />
              </div>
              <div className="ribbon__row">
                <Btn small icon="⇤" label="왼쪽에 삽입" disabled={!cell}
                  onClick={() => editTable('insert', { axis: 'col', col: cell.col })} />
                <Btn small icon="⇥" label="오른쪽에 삽입" disabled={!cell}
                  onClick={() => editTable('insert', { axis: 'col', col: cell.col + 1 })} />
              </div>
            </div>
          </Group>

          <Group label="삭제">
            <div className="ribbon__stack">
              <Btn small icon="⌦" label="행 삭제" disabled={!cell}
                onClick={() => editTable('delete', { axis: 'row', row: cell.row })} />
              <Btn small icon="⌫" label="열 삭제" disabled={!cell}
                onClick={() => editTable('delete', { axis: 'col', col: cell.col })} />
            </div>
          </Group>

          <Group label="병합">
            <div className="ribbon__stack">
              <Btn small icon="⊞" label="오른쪽과 병합" disabled={!cell}
                onClick={() => editTable('merge', { col: cell.col, row: cell.row, col2: cell.col + 1, row2: cell.row })} />
              <Btn small icon="⊟" label="아래와 병합" disabled={!cell}
                onClick={() => editTable('merge', { col: cell.col, row: cell.row, col2: cell.col, row2: cell.row + 1 })} />
              <Btn small icon="⊠" label="병합 해제" disabled={!cell}
                onClick={() => editTable('split', { col: cell.col, row: cell.row })} />
            </div>
          </Group>

          <Group label="맞춤">
            <div className="ribbon__stack">
              <div className="ribbon__row">
                {[['left', '⟵'], ['center', '↔'], ['right', '⟶']].map(([value, icon]) => (
                  <Btn key={value} small icon={icon} label="" title={`가로 ${value}`} disabled={!cell}
                    pressed={cellFormatOf(selected, cell)?.align === value}
                    onClick={() => editTable('format', { col: cell.col, row: cell.row, align: value })} />
                ))}
              </div>
              <div className="ribbon__row">
                {[['top', '⤒'], ['middle', '⇕'], ['bottom', '⤓']].map(([value, icon]) => (
                  <Btn key={value} small icon={icon} label="" title={`세로 ${value}`} disabled={!cell}
                    pressed={cellFormatOf(selected, cell)?.valign === value}
                    onClick={() => editTable('format', { col: cell.col, row: cell.row, valign: value })} />
                ))}
              </div>
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
          <Group label="구조">
            <div style={{ fontSize: 11.5, color: 'var(--ink-2)', maxWidth: 440, lineHeight: 1.6, padding: '2px 4px' }}>
              제목 {outline.length}개 · 블록 {section.blocks.length}개 · 서식 지정된 문단{' '}
              {section.blocks.filter((b) => b.override).length}개 (이 문단들만 <code>.meta.json</code>에 기록됩니다)
            </div>
          </Group>
        </>
      )}
    </Ribbon>
  );

  /* ----------------------------------------------------------------- render */

  // `key` is passed explicitly at the call site: React refuses to accept it via a
  // spread, and doing so silently breaks reconciliation.
  const blockProps = (block) => ({
    block,
    zoom,
    folder: project.folder,
    selected: selectedId === block.id,
    editing: editingId === block.id,
    focusRequest: focusRequest?.id === block.id ? focusRequest : null,
    onFocusHandled: () => setFocusRequest(null),
    onSelect: () => {
      // With the brush loaded, the next paragraph clicked gets the formatting
      // instead of being opened for typing — Word's one-shot behaviour.
      if (painter !== null) {
        setSelectedId(block.id);
        paintOnto(block.id);
        return;
      }
      setSelectedId(block.id);
      // A table is edited cell by cell, not as a block of markdown in a
      // textarea — the same distinction Office makes.
      if (block.type === 'table') {
        setEditingId(null);
        return;
      }
      if (openBlock(block)) return;
      if (TEXTLESS_TYPES.has(block.type)) {
        setEditingId(null);
        return;
      }
      setEditingId(block.id);
      // An empty paragraph has no text to click into; put the caret there.
      if (!String(block.md ?? '').trim()) setFocusRequest({ id: block.id, caret: 0 });
    },
    onChange: (md) => setBlockMd(block.id, md),
    onSplit: (caret) => splitBlock(block.id, caret),
    onMergeBackward: (caretInMd) => mergeBackward(block.id, caretInMd),
    onStepParagraph: (direction) => stepParagraph(block.id, direction),
    onIndent: (direction) => stepIndent(direction),
    onUndo: undo,
    onRedo: redo,
    onExit: () => setEditingId(null),
    onContextMenu: (e) => {
      setSelectedId(block.id);
      ctx.open(e, blockMenu(block));
    },
    highlight: find?.query ?? '',
    activeCell: activeCell?.id === block.id ? activeCell : null,
    onActiveCellChange: (next) => setActiveCell(next ? { ...next, id: block.id } : null),
    onChangeTable:
      block.type === 'table'
        ? (md, table) => patchBlock(block.id, { md, table })
        : undefined,
  });

  return (
    <Shell
      type="doc"
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
          <span>단어 {stats.words}</span>
          <span>글자 {stats.chars}</span>
          <span>
            {pages.length}페이지 · 섹션 {index + 1}/{sections.length}
          </span>
          {selected?.override && <span>이 문단은 JSON에 서식이 기록됩니다</span>}
          <span className="statusbar__spacer" />
          <ZoomSlider value={zoom} onChange={setZoom} min={0.5} max={1.5} />
        </>
      }
    >
      <aside className="panel panel--left">
        <div className="panel__head">
          <span>탐색</span>
        </div>
        <div className="panel__body">
          <div className="panel__section">
            <p className="panel__title">섹션</p>
            {sections.map((s, i) => (
              <button key={s.id ?? i} className="outline-item" aria-current={i === index} onClick={() => setSectionIndex(i)}>
                {i + 1}. {s.name}
              </button>
            ))}
          </div>
          <div className="panel__section">
            <p className="panel__title">개요</p>
            {outline.length === 0 ? (
              <p style={{ fontSize: 12, color: 'var(--ink-3)', margin: 0 }}>제목을 추가하면 표시됩니다.</p>
            ) : (
              outline.map((o) => (
                <button
                  key={o.id}
                  className="outline-item"
                  style={{ paddingLeft: 10 + (o.level - 1) * 12 }}
                  aria-current={o.id === selectedId}
                  onClick={() => {
                    setSelectedId(o.id);
                    document.getElementById(`block-${o.id}`)?.scrollIntoView({ block: 'center' });
                  }}
                >
                  {o.text}
                </button>
              ))
            )}
          </div>
        </div>
      </aside>

      <div className="stage">
        {find && (
          <FindBar
            state={find}
            hits={findHits}
            onChange={(patch) => setFind((f) => ({ ...f, ...patch, at: patch.query !== undefined ? null : f.at }))}
            onNext={() => stepFind(1)}
            onPrev={() => stepFind(-1)}
            onReplace={replaceCurrent}
            onReplaceAll={replaceAll}
            onClose={() => setFind(null)}
          />
        )}

        <div className="doc-scroll">
          {pages.map((pageBlocks, pageNumber) => (
            <div
              key={pageNumber}
              className="page"
              style={{
                width: pageSize.w * zoom,
                minHeight: pageSize.h * zoom,
                height: pageSize.h * zoom,
              }}
              onMouseDown={(e) => e.target === e.currentTarget && setSelectedId(null)}
              onContextMenu={(e) => {
                if (e.target !== e.currentTarget) return;
                ctx.open(e, [
                  { label: '문단 추가', onClick: () => insertBlock(section.blocks[section.blocks.length - 1]?.id, '') },
                  { label: '인쇄', shortcut: 'Ctrl+P', onClick: () => window.print() },
                ]);
              }}
            >
              <div
                className="page__inner md md--doc"
                style={{
                  padding: `${(page.margin?.top ?? 72) * zoom}px ${(page.margin?.right ?? 72) * zoom}px ${
                    (page.margin?.bottom ?? 72) * zoom
                  }px ${(page.margin?.left ?? 72) * zoom}px`,
                  fontSize: `${15 * zoom}px`,
                  // A multi-column section flows its text the way Word does.
                  columnCount: columns > 1 ? columns : undefined,
                  columnGap: columns > 1 ? COLUMN_GAP * zoom : undefined,
                }}
                onDoubleClick={(e) => {
                  // Double-clicking open page space starts a new paragraph at
                  // the end of this page — the free-form reflex of "click where
                  // the text should go".
                  if (e.target !== e.currentTarget) return;
                  const last = pageBlocks[pageBlocks.length - 1];
                  insertBlock(last?.id ?? null, '');
                }}
              >
                {pageBlocks.map((block) => (
                  <DocBlock key={block.id} {...blockProps(block)} />
                ))}

                {pageNumber === pages.length - 1 && (
                  <button
                    className="outline-item"
                    style={{ marginTop: 12, color: 'var(--ink-3)' }}
                    onClick={() => insertBlock(section.blocks[section.blocks.length - 1]?.id, '')}
                  >
                    + 문단 추가
                  </button>
                )}
              </div>
              {/* Real running heads: they print, and their page tokens resolve
                  per page the way Word's fields do. */}
              <RunningBand
                running={page.header}
                where="header"
                page={pageNumber + 1}
                pages={pages.length}
                margin={page.margin}
                zoom={zoom}
              />
              <RunningBand
                running={page.footer}
                where="footer"
                page={pageNumber + 1}
                pages={pages.length}
                margin={page.margin}
                zoom={zoom}
              />
              {/* An editing aid, not part of the document — hidden when printing. */}
              <div className="pagenum">
                {pageNumber + 1} / {pages.length}
              </div>
            </div>
          ))}

          {/* Explicit breaks are shown between pages so they can be removed. */}
          {section.blocks.filter(isPageBreak).length > 0 && (
            <div style={{ fontSize: 11, color: 'var(--ink-3)' }}>
              페이지 나누기 {section.blocks.filter(isPageBreak).length}개 — 우클릭 메뉴에서 삭제할 수 있습니다
            </div>
          )}
        </div>

        {/*
          Hidden measuring layer: the same markup at the same width, rendered once so
          pagination can use real heights instead of guessing.
        */}
        <div className="measure-layer md md--doc" style={{ width: columnWidth * zoom, fontSize: `${15 * zoom}px` }}>
          {section.blocks.filter((b) => !isPageBreak(b)).map((block) => (
            <div
              key={block.id}
              className={block.override?.style?.fontSize ? 'md--sized' : undefined}
              style={blockStyle(block, zoom)}
              ref={(node) => {
                if (node) measureRefs.current.set(block.id, node);
                else measureRefs.current.delete(block.id);
              }}
            >
              <BlockContent block={block} folder={project.folder} width={columnWidth * zoom} />
            </div>
          ))}
        </div>
      </div>

      {inspectorOpen && <FileInspector project={project} activeIndex={index} onClose={() => setInspectorOpen(false)} />}

      {ctx.menu && <ContextMenu {...ctx.menu} onClose={ctx.close} />}

      {imageDialog && (
        <ImageDialog
          folder={project.folder}
          initial={imageDialog.id ? { src: imageDialog.src, alt: imageDialog.alt } : null}
          notify={notify}
          onCancel={() => setImageDialog(null)}
          onConfirm={({ src, alt }) => {
            const md = `![${alt || '이미지'}](${src})`;
            if (imageDialog.id) setBlockMd(imageDialog.id, md);
            else insertBlock(selectedId, md, { focus: false });
            setImageDialog(null);
          }}
        />
      )}

      {linkDialog && (
        <Dialog
          title="하이퍼링크"
          confirmLabel="넣기"
          onCancel={() => setLinkDialog(null)}
          onConfirm={() => {
            const url = linkDialog.url.trim();
            if (!url) return;
            const label = linkDialog.text.trim() || url;
            // A markdown link, so the address stays visible in the .md file and
            // becomes a real Word hyperlink on export.
            const block = section.blocks.find((b) => b.id === selectedId);
            const md = `[${label}](${url})`;
            if (block) setBlockMd(block.id, `${block.md ?? ''}${block.md?.trim() ? ' ' : ''}${md}`);
            else insertBlock(selectedId, md);
            setLinkDialog(null);
          }}
        >
          <p>
            선택한 문단 끝에 <code>[표시할 글](주소)</code> 형태로 들어갑니다. 내보낼 때 Word의 진짜
            하이퍼링크가 됩니다.
          </p>
          <Field label="표시할 글">
            <input
              value={linkDialog.text}
              onChange={(e) => setLinkDialog({ ...linkDialog, text: e.target.value })}
              placeholder="비우면 주소가 그대로 보입니다"
            />
          </Field>
          <Field label="주소">
            <input
              value={linkDialog.url}
              onChange={(e) => setLinkDialog({ ...linkDialog, url: e.target.value })}
              placeholder="https://"
            />
          </Field>
        </Dialog>
      )}

      {runningDialog && (
        <RunningDialog
          where={runningDialog}
          initial={runningDialog === 'header' ? page.header : page.footer}
          onCancel={() => setRunningDialog(null)}
          onConfirm={(value) => {
            setRunning(runningDialog, value);
            setRunningDialog(null);
          }}
        />
      )}

      {chartDialog && (
        <ChartDialog
          initial={chartDialog.spec}
          onCancel={() => setChartDialog(null)}
          onConfirm={(spec) => {
            const md = serializeChartBlock(spec);
            if (chartDialog.id) setBlockMd(chartDialog.id, md);
            else insertBlock(selectedId, md, { focus: false });
            setChartDialog(null);
          }}
        />
      )}
    </Shell>
  );
}

/* ---------------------------------------------------------------- doc block */

function DocBlock({
  block, folder, selected, editing, focusRequest, onFocusHandled, highlight,
  onSelect, onChange, onSplit, onMergeBackward, onExit, onContextMenu,
  onStepParagraph, onIndent, onUndo, onRedo,
  activeCell, onActiveCellChange, onChangeTable, zoom = 1,
}) {
  const style = blockStyle(block, zoom);

  // Tables, charts, images, page breaks and rules are not paragraphs — they
  // are selected, and edited through their own surfaces (dialog, table grid).
  if (TEXTLESS_TYPES.has(block.type)) {
    return (
      <div
        id={`block-${block.id}`}
        className={`docblock${selected ? ' is-selected' : ''}`}
        style={style}
        onClick={() => !editing && onSelect()}
        onContextMenu={(e) => {
          e.preventDefault();
          onContextMenu(e);
        }}
      >
        <BlockContent
          block={block}
          folder={folder}
          highlight={highlight}
          activeCell={activeCell}
          onActiveCellChange={onActiveCellChange}
          onChangeTable={onChangeTable}
          onSelectBlock={onSelect}
        />
        {selected && block.override && <span className="docblock__badge">meta.json</span>}
      </div>
    );
  }

  return (
    <div
      id={`block-${block.id}`}
      className={`docblock${selected ? ' is-selected' : ''}${editing ? ' is-editing' : ''}${
        block.override?.style?.fontSize ? ' md--sized' : ''
      }`}
      style={style}
      onClick={() => !editing && onSelect()}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu(e);
      }}
    >
      {/*
        The paragraph is always drawn the way it reads: rendered markdown
        becomes the editable surface, so `#` and `**` never show up — typing
        `# ` + space turns the paragraph into a heading, and the stored text is
        recovered from the DOM. Clicking anywhere lands the caret where the
        mouse is, the way a word processor's page does.
      */}
      <MarkdownEditor
        className={`docblock__editor md md--doc${block.override?.style?.fontSize ? ' md--sized' : ''}`}
        editable={editing}
        alwaysEditable
        value={block.md ?? ''}
        highlight={highlight}
        assetResolver={(src) => (isProjectAsset(src) ? assetUrl(folder, src) : src)}
        focusRequest={focusRequest}
        onFocusHandled={onFocusHandled}
        onInput={onChange}
        onSplit={onSplit}
        onMergeBackward={onMergeBackward}
        onStepParagraph={onStepParagraph}
        onIndent={onIndent}
        onUndo={onUndo}
        onRedo={onRedo}
        onExit={onExit}
        onContextMenu={(e) => {
          e.preventDefault();
          onContextMenu(e);
        }}
      />
      {selected && block.override && <span className="docblock__badge">meta.json</span>}
    </div>
  );
}
/** Edit one header or footer: three slots and the tokens they accept. */
function RunningDialog({ where, initial, onCancel, onConfirm }) {
  const [value, setValue] = useState({
    left: initial?.left ?? '',
    center: initial?.center ?? '',
    right: initial?.right ?? '',
  });
  const set = (slot) => (e) => setValue((v) => ({ ...v, [slot]: e.target.value }));
  return (
    <Dialog
      title={where === 'header' ? '머리글' : '바닥글'}
      onCancel={onCancel}
      onConfirm={() => onConfirm(value)}
    >
      <Field label="왼쪽">
        <input value={value.left} onChange={set('left')} placeholder="예: 회사명" />
      </Field>
      <Field label="가운데">
        <input value={value.center} onChange={set('center')} placeholder="예: {PAGE} / {PAGES}" />
      </Field>
      <Field label="오른쪽">
        <input value={value.right} onChange={set('right')} placeholder="예: {DATE}" />
      </Field>
      <p className="hint">
        <code>{'{PAGE}'}</code> 현재 페이지 · <code>{'{PAGES}'}</code> 전체 페이지 ·{' '}
        <code>{'{DATE}'}</code> 날짜. 내보낼 때 Word의 필드가 되므로 페이지가 늘어나도 맞습니다.
        비우면 머리글·바닥글이 사라집니다.
      </p>
    </Dialog>
  );
}

/**
 * A header or footer, drawn inside the page's margin.
 *
 * The three slots are laid out left, centre and right — the same three Word
 * writes between tab stops, which is why an imported header keeps its position.
 */
function RunningBand({ running, where, page, pages, margin, zoom }) {
  if (!running || (!running.left && !running.center && !running.right)) return null;
  const today = new Date().toISOString().slice(0, 10);
  const slots = resolveRunning(running, page, pages, today);
  const inset = {
    left: (margin?.left ?? 72) * zoom,
    right: (margin?.right ?? 72) * zoom,
    // Halfway into the margin, which is where Word puts them by default.
    [where === 'header' ? 'top' : 'bottom']: Math.max(
      8,
      ((where === 'header' ? margin?.top : margin?.bottom) ?? 72) * 0.35 * zoom
    ),
  };
  return (
    <div className={`running running--${where}`} style={{ ...inset, fontSize: 11 * zoom }}>
      {slots.map((text, i) => (
        <span key={i} className="running__slot">
          {text}
        </span>
      ))}
    </div>
  );
}

/**
 * One block's own formatting, in the same shape for the page and for the hidden
 * measuring layer.
 *
 * Both have to agree or a page break lands somewhere the reader does not see it,
 * and both have to scale with the zoom for the same reason.
 */
function blockStyle(block, zoom = 1) {
  const override = block.override ?? {};
  const size = override.style?.fontSize;
  return {
    textAlign: override.align,
    marginLeft: override.indent ? `${override.indent * zoom}px` : undefined,
    marginTop: override.spacing?.before ? `${override.spacing.before * zoom}px` : undefined,
    marginBottom: override.spacing?.after ? `${override.spacing.after * zoom}px` : undefined,
    color: override.style?.color,
    lineHeight: override.style?.lineHeight,
    fontSize: size ? `${size * zoom}px` : undefined,
    fontStyle: override.style?.italic ? 'italic' : undefined,
    fontWeight: override.style?.bold ? 700 : undefined,
    textDecoration: override.style?.underline ? 'underline' : undefined,
    background: override.style?.bg,
  };
}

/** Rendered view of one block: chart, table, image, or markdown. */
function BlockContent({
  block, folder, width, highlight,
  activeCell, onActiveCellChange, onChangeTable, onSelectBlock,
}) {
  const chart = parseChartBlock(block.md);
  if (chart) {
    return <ChartView spec={chart} width={width ?? 620} height={Math.round((width ?? 620) * 0.55)} />;
  }
  // A table draws from its layout, so an imported one keeps its column widths,
  // merges and header band instead of collapsing to a plain markdown grid.
  if (block.type === 'table' && block.table) {
    return (
      <TableView
        md={block.md}
        spec={block.table}
        width={width ?? 620}
        active={activeCell}
        onActiveChange={onActiveCellChange}
        onChange={onChangeTable}
        onSelectBlock={onSelectBlock}
      />
    );
  }
  if (!block.md?.trim()) return <p style={{ color: '#b9b7b5' }}>빈 문단</p>;

  const html = renderMarkdown(block.md, {
    assetResolver: (src) => (isProjectAsset(src) ? assetUrl(folder, src) : src),
  });
  return <div dangerouslySetInnerHTML={{ __html: highlight ? markHtml(html, highlight) : html }} />;
}

/* -------------------------------------------------------------------- utils */

function classify(md) {
  const t = String(md ?? '').trim();
  if (t === PAGE_BREAK) return 'pagebreak';
  if (/^```chart/.test(t)) return 'chart';
  if (/^#{1,6}\s/.test(t)) return 'heading';
  if (/^(```|~~~)/.test(t)) return 'code';
  if (/^\|/.test(t)) return 'table';
  if (/^>/.test(t)) return 'quote';
  if (/^([-+*]|\d+[.)])\s/.test(t)) return 'list';
  if (/^(---+|\*\*\*+)$/.test(t)) return 'hr';
  if (/^!\[[^\]]*\]\([^)]*\)$/.test(t)) return 'image';
  return 'paragraph';
}

function styleOf(md) {
  const level = headingLevel(md);
  return level > 0 && level <= 3 ? String(level) : 'body';
}

function prevIdOf(section, id) {
  const at = section.blocks.findIndex((b) => b.id === id);
  return at > 0 ? section.blocks[at - 1].id : undefined;
}

function escapeRe(text) {
  return String(text).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

