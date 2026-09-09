import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { indexToCol, toRef, parseRange, displayValue, dependencies, LIMITS } from '../core/index.js';
import { normalizeRange, mergeCovering, fillTarget, cycleRefLocks } from './gridOps.js';
import SheetCharts from './SheetCharts.jsx';
import { borderStyles } from '../lib/borderStyle.js';

const ROW_BUFFER = 24;
const MIN_VISIBLE_ROWS = 32;
const HEADER_H = 26;
const ROW_HEAD_W = 44;

/**
 * The spreadsheet surface.
 *
 * Rows render lazily — used range plus a buffer — because a 200x26 sheet is 5,200
 * DOM nodes and almost all of them are empty. The window grows as the selection
 * moves down, which keeps typing responsive without a full virtualiser.
 *
 * The interactions here are the ones Excel users have in muscle memory: drag to
 * select, drag the corner handle to fill, drag a header edge to resize,
 * double-click an edge to auto-fit, right-click for a menu, and frozen panes that
 * actually stay put while scrolling.
 */
export default function Sheet({
  sheet, sel, onSelChange, editing, showFormulas, zoom = 1,
  onEditStart, onCommit, onEditCancel,
  onFill, onResizeCol, onResizeRow, onAutoFitCol, onAutoFitRow, onContextMenu,
  findHits, currentHit,
  selectedChartId, onSelectChart, onMoveChart, onEditChart, onDeleteChart,
}) {
  const containerRef = useRef(null);
  const [dragSelect, setDragSelect] = useState(false);
  /** `'row'` or `'col'` while a drag across the headers is selecting. */
  const [headerDrag, setHeaderDrag] = useState(null);
  const [fillTo, setFillTo] = useState(null);
  const [resizing, setResizing] = useState(null);

  /*
   * Live drag state is mirrored into refs.
   *
   * A `setState` updater must be pure — React may call it more than once — so the
   * commit at mouse-up reads the ref and calls the parent from the event handler
   * instead of from inside the updater, where the nested state update was
   * unreliable.
   */
  const resizingRef = useRef(null);
  const fillRef = useRef(null);
  resizingRef.current = resizing;
  fillRef.current = fillTo;

  const range = normalizeRange(sel);
  const usedRow = maxUsedRow(sheet.cells);
  const visibleRows = Math.min(
    sheet.dims.rows,
    Math.max(MIN_VISIBLE_ROWS, usedRow + ROW_BUFFER, range.r2 + ROW_BUFFER)
  );
  const visibleCols = Math.min(sheet.dims.cols, Math.max(12, maxUsedCol(sheet.cells) + 4, range.c2 + 3));

  // Zoom scales the rendered geometry instead of CSS-transforming the table:
  // a transform would break `position: sticky` on the frozen panes.
  const z = zoom;
  const px = (n) => Math.round(n * z);
  const colWidth = (c) => sheet.colWidths?.[indexToCol(c)] ?? LIMITS.colWidth;
  const rowHeight = (r) => sheet.rowHeights?.[String(r + 1)] ?? LIMITS.rowHeight;
  const headerH = px(HEADER_H);
  const rowHeadW = px(ROW_HEAD_W);

  const frozenCols = Math.min(sheet.frozen?.cols ?? 0, visibleCols);
  const frozenRows = Math.min(sheet.frozen?.rows ?? 0, visibleRows);

  /** Sticky offsets for frozen panes, accumulated from the real sizes. */
  const colOffset = useMemo(() => {
    const offsets = [];
    let x = rowHeadW;
    for (let c = 0; c < frozenCols; c++) {
      offsets.push(x);
      x += px(colWidth(c));
    }
    return offsets;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [frozenCols, sheet.colWidths, z]);

  const rowOffset = useMemo(() => {
    const offsets = [];
    let y = headerH;
    for (let r = 0; r < frozenRows; r++) {
      offsets.push(y);
      y += px(rowHeight(r));
    }
    return offsets;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [frozenRows, sheet.rowHeights, z]);

  /** Cells the selected formula reads, highlighted like Excel's trace precedents. */
  const depRefs = useMemo(() => {
    if (editing || range.r1 !== range.r2 || range.c1 !== range.c2) return new Set();
    const cell = sheet.cells[toRef(range.c1, range.r1)];
    return cell?.f ? new Set(dependencies(cell.f)) : new Set();
  }, [sheet.cells, range.r1, range.c1, range.r2, range.c2, editing]);

  /** The spill range the selected cell belongs to — anchor or ghost — outlined
   *  the way Excel frames a dynamic array when you land inside one. */
  const spillRefs = useMemo(() => {
    if (editing || range.r1 !== range.r2 || range.c1 !== range.c2) return new Set();
    const cell = sheet.cells[toRef(range.c1, range.r1)];
    const anchor = cell?.spill ? cell : sheet.cells[cell?.spillFrom ?? ''];
    const spec = anchor?.spill;
    if (!spec) return new Set();
    const r = parseRange(spec);
    if (!r) return new Set();
    const out = new Set();
    for (let row = r.start.row; row <= r.end.row; row++) {
      for (let col = r.start.col; col <= r.end.col; col++) out.add(toRef(col, row));
    }
    return out;
  }, [sheet.cells, range.r1, range.c1, range.r2, range.c2, editing]);

  const hitSet = useMemo(() => new Set((findHits ?? []).map((h) => h.ref)), [findHits]);

  // Keep the active cell in view when navigating by keyboard.
  useEffect(() => {
    if (editing) return;
    const el = containerRef.current?.querySelector('td.is-selected');
    el?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }, [sel.row, sel.col, editing]);

  /* ------------------------------------------------------------- resizing */

  useEffect(() => {
    if (!resizing) return undefined;
    const onMove = (e) => {
      const current = resizingRef.current;
      if (!current) return;
      const delta = (current.axis === 'col' ? e.clientX - current.startX : e.clientY - current.startY) / zoom;
      const size = Math.max(current.axis === 'col' ? 24 : 16, current.origin + delta);
      resizingRef.current = { ...current, size };
      setResizing(resizingRef.current);
    };
    const onUp = () => {
      const current = resizingRef.current;
      setResizing(null);
      resizingRef.current = null;
      if (!current || current.size === current.origin) return;
      if (current.axis === 'col') onResizeCol(current.index, current.size);
      else onResizeRow(current.index, current.size);
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    return () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
  }, [resizing, zoom, onResizeCol, onResizeRow]);

  const liveColWidth = (c) => (resizing?.axis === 'col' && resizing.index === c ? resizing.size : colWidth(c));
  const liveRowHeight = (r) => (resizing?.axis === 'row' && resizing.index === r ? resizing.size : rowHeight(r));

  /* ------------------------------------------------------------ fill drag */

  useEffect(() => {
    if (!fillTo) return undefined;
    const onUp = () => {
      const current = fillRef.current;
      setFillTo(null);
      fillRef.current = null;
      if (current?.target) onFill(current.source, current.target);
    };
    const onKey = (e) => {
      if (e.key !== 'Escape') return;
      setFillTo(null);
      fillRef.current = null;
    };
    window.addEventListener('mouseup', onUp);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mouseup', onUp);
      window.removeEventListener('keydown', onKey);
    };
  }, [fillTo, onFill]);

  const fillPreview = fillTo?.target ?? null;
  const inFillPreview = (r, c) =>
    fillPreview &&
    r >= fillPreview.r1 && r <= fillPreview.r2 && c >= fillPreview.c1 && c <= fillPreview.c2 &&
    !(r >= range.r1 && r <= range.r2 && c >= range.c1 && c <= range.c2);

  /* ---------------------------------------------------------------- cells */

  const totalWidth = rowHeadW + Array.from({ length: visibleCols }, (_, c) => px(liveColWidth(c))).reduce((a, b) => a + b, 0);

  /**
   * The selection marquee's rectangle, in the table's own coordinates.
   *
   * Drawn as one overlay element instead of per-cell shadows: a single border
   * keeps the four sides crisp and exactly as thick as Excel's, and rides
   * above the cells' own grid lines, which would otherwise wash the frame out.
   * Summing real column/row sizes keeps it glued to the cells at any zoom.
   */
  const selBox = useMemo(() => {
    let left = rowHeadW;
    let top = headerH;
    let width = 0;
    let height = 0;
    for (let c = 0; c < range.c1; c++) left += px(colWidth(c));
    for (let r = 0; r < range.r1; r++) top += px(rowHeight(r));
    for (let c = range.c1; c <= range.c2; c++) width += px(colWidth(c));
    for (let r = range.r1; r <= range.r2; r++) height += px(rowHeight(r));
    return { left, top, width, height };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [range, z, sheet.colWidths, sheet.rowHeights]);

  const enterCell = (r, c) => {
    const current = fillRef.current;
    if (current) {
      const target = fillTarget(current.source, r, c);
      fillRef.current = { ...current, target };
      setFillTo(fillRef.current);
      return;
    }
    if (dragSelect) onSelChange({ ...sel, row2: r, col2: c });
  };

  return (
    <div
      className="sheet"
      ref={containerRef}
      onMouseUp={() => {
        setDragSelect(false);
        setHeaderDrag(null);
      }}
      onMouseLeave={() => {
        setDragSelect(false);
        setHeaderDrag(null);
      }}
    >
      <div style={{ position: 'relative', width: totalWidth }}>
      <table style={{ width: totalWidth, fontSize: `${Math.max(8, Math.round(LIMITS.cellPx * z))}px` }}>
        <colgroup>
          <col style={{ width: rowHeadW }} />
          {Array.from({ length: visibleCols }, (_, c) => (
            <col key={c} style={{ width: px(liveColWidth(c)) }} />
          ))}
        </colgroup>

        <thead>
          <tr style={{ height: headerH }}>
            <th style={{ left: 0, zIndex: 6 }} aria-label="모두 선택">
              <button
                type="button"
                className="selectall"
                title="시트 전체 선택"
                onClick={() => onSelChange({ row: 0, col: 0, row2: visibleRows - 1, col2: visibleCols - 1 })}
              />
            </th>
            {Array.from({ length: visibleCols }, (_, c) => {
              const frozen = c < frozenCols;
              return (
                <th
                  key={c}
                  className={[
                    c >= range.c1 && c <= range.c2 ? 'is-active' : '',
                    frozen ? 'is-frozen' : '',
                  ].filter(Boolean).join(' ')}
                  style={frozen ? { left: colOffset[c], zIndex: 5 } : undefined}
                  onMouseDown={(e) => {
                    if (e.button !== 0) return;
                    if (e.shiftKey) onSelChange({ ...sel, row2: visibleRows - 1, col2: c });
                    else onSelChange({ row: 0, col: c, row2: visibleRows - 1, col2: c });
                    setHeaderDrag('col');
                  }}
                  onMouseEnter={() => {
                    if (headerDrag === 'col') onSelChange({ ...sel, row2: visibleRows - 1, col2: c });
                  }}
                  onContextMenu={(e) => onContextMenu?.(e, { kind: 'col', index: c })}
                  title={`${indexToCol(c)}열 — 눌러 전체 선택, 끌어 여러 열 선택, 경계를 끌어 너비 조절`}
                >
                  {indexToCol(c)}
                  <span
                    className="resizer resizer--col"
                    onMouseDown={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      resizingRef.current = { axis: 'col', index: c, startX: e.clientX, origin: colWidth(c), size: colWidth(c) };
                      setResizing(resizingRef.current);
                    }}
                    onDoubleClick={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      onAutoFitCol?.(c);
                    }}
                    title="너비 조절 (두 번 누르면 자동 맞춤)"
                  />
                </th>
              );
            })}
          </tr>
        </thead>

        <tbody>
          {Array.from({ length: visibleRows }, (_, r) => {
            const frozenRow = r < frozenRows;
            return (
              <tr key={r} style={{ height: px(liveRowHeight(r)) }}>
                <th
                  className={[
                    r >= range.r1 && r <= range.r2 ? 'is-active' : '',
                    frozenRow ? 'is-frozen' : '',
                  ].filter(Boolean).join(' ')}
                  style={{ left: 0, ...(frozenRow ? { top: rowOffset[r], zIndex: 4 } : {}) }}
                  onMouseDown={(e) => {
                    if (e.button !== 0) return;
                    // Shift extends from the anchor row, and a plain press
                    // starts a drag across the row headers — both of which are
                    // how a block of rows gets selected before an insert.
                    if (e.shiftKey) onSelChange({ ...sel, row2: r, col2: visibleCols - 1 });
                    else onSelChange({ row: r, col: 0, row2: r, col2: visibleCols - 1 });
                    setHeaderDrag('row');
                  }}
                  onMouseEnter={() => {
                    if (headerDrag === 'row') onSelChange({ ...sel, row2: r, col2: visibleCols - 1 });
                  }}
                  onContextMenu={(e) => onContextMenu?.(e, { kind: 'row', index: r })}
                  title={`${r + 1}행 — 눌러 전체 선택, 끌어 여러 행 선택, 경계를 끌어 높이 조절`}
                >
                  {r + 1}
                  <span
                    className="resizer resizer--row"
                    onMouseDown={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      resizingRef.current = { axis: 'row', index: r, startY: e.clientY, origin: rowHeight(r), size: rowHeight(r) };
                      setResizing(resizingRef.current);
                    }}
                    onDoubleClick={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      onAutoFitRow?.(r);
                    }}
                    title="높이 조절 (두 번 누르면 자동 맞춤)"
                  />
                </th>

                {Array.from({ length: visibleCols }, (_, c) => {
                  const merge = mergeCovering(sheet, r, c);
                  // A cell covered by a merge but not its anchor renders nothing.
                  if (merge && !(merge.r1 === r && merge.c1 === c)) return null;

                  const ref = toRef(c, r);
                  const cell = sheet.cells[ref];
                  const inSel =
                    r >= range.r1 && r <= range.r2 && c >= range.c1 && c <= range.c2;
                  const isSelected = r === sel.row && c === sel.col;
                  const inRange = inSel && !isSelected;
                  // The line toward the next column/row is an interior line of
                  // the selection only when that neighbour is selected too —
                  // darkened so the cell count stays readable inside the fill.
                  const rightIn = inSel && c + 1 <= range.c2;
                  const belowIn = inSel && r + 1 <= range.r2;
                  const isEditing = editing && isSelected;
                  const frozenCol = c < frozenCols;
                  const isFillCorner = !editing && r === range.r2 && c === range.c2;

                  return (
                    <td
                      key={c}
                      rowSpan={merge ? merge.r2 - merge.r1 + 1 : undefined}
                      colSpan={merge ? merge.c2 - merge.c1 + 1 : undefined}
                      className={[
                        isSelected ? 'is-selected' : '',
                        inRange ? 'is-inrange' : '',
                        rightIn ? 'is-rightin' : '',
                        belowIn ? 'is-belowin' : '',
                        depRefs.has(ref) ? 'is-dep' : '',
                        spillRefs.has(ref) && !isSelected ? 'is-spill' : '',
                        inFillPreview(r, c) ? 'is-fillpreview' : '',
                        frozenCol || frozenRow ? 'is-frozen' : '',
                      ].filter(Boolean).join(' ')}
                      style={{
                        ...cellVisualStyle(cell),
                        ...(frozenCol ? { left: colOffset[c] } : {}),
                        ...(frozenRow ? { top: rowOffset[r] } : {}),
                        ...(frozenCol || frozenRow ? { position: 'sticky' } : {}),
                        ...(frozenCol && frozenRow ? { zIndex: 3 } : {}),
                      }}
                      onMouseDown={(e) => {
                        if (e.button !== 0) return;
                        setDragSelect(true);
                        onSelChange(
                          e.shiftKey
                            ? { ...sel, row2: r, col2: c }
                            : { row: r, col: c, row2: r, col2: c }
                        );
                      }}
                      onMouseEnter={() => enterCell(r, c)}
                      onDoubleClick={() => onEditStart(r, c)}
                      onContextMenu={(e) => {
                        if (!(r >= range.r1 && r <= range.r2 && c >= range.c1 && c <= range.c2)) {
                          onSelChange({ row: r, col: c, row2: r, col2: c });
                        }
                        onContextMenu?.(e, { kind: 'cell', row: r, col: c });
                      }}
                    >
                      {isEditing ? (
                        <CellEditor initial={editing.value} onCommit={onCommit} onCancel={onEditCancel} />
                      ) : (
                        <CellView
                          cell={cell}
                          showFormulas={showFormulas}
                          highlight={hitSet.has(ref) ? { query: findHits?.query, current: currentHit === ref } : null}
                        />
                      )}

                      {isFillCorner && (
                        <span
                          className="fillhandle"
                          title="끌어서 자동 채우기"
                          onMouseDown={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            fillRef.current = { source: { ...range }, target: null };
                            setFillTo(fillRef.current);
                          }}
                        />
                      )}
                    </td>
                  );
                })}
              </tr>
            );
          })}
        </tbody>
      </table>

      {/* Excel's selection frame: one dark rectangle around the whole range,
          above the cells so its sides never lose to their grid lines. */}
      <div
        className="selframe"
        aria-hidden="true"
        style={{ left: selBox.left, top: selBox.top, width: selBox.width, height: selBox.height }}
      />

      <SheetCharts
        sheet={sheet}
        zoom={z}
        selectedId={selectedChartId}
        onSelect={onSelectChart}
        onMove={onMoveChart}
        onEdit={onEditChart}
        onDelete={onDeleteChart}
      />
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------- cell */

function CellView({ cell, showFormulas, highlight }) {
  if (!cell) return <div className="cell" />;
  const text = showFormulas && cell.f ? cell.f : displayValue(cell);
  const type = cell.t === 'e' ? 'e' : cell.t === 'n' || cell.t === 'd' ? 'n' : cell.t === 'b' ? 'b' : 's';
  const style = cell.style ?? {};
  return (
    <div
      className={`cell cell--${showFormulas && cell.f ? 's' : type}${style.wrap ? ' cell--wrap' : ''}`}
      style={{
        // Relative to the sheet's own text, so an imported workbook's 16pt
        // heading stays 16pt-shaped at every zoom without being told the zoom.
        fontSize: style.fontSize ? `${(style.fontSize / LIMITS.cellPx).toFixed(3)}em` : undefined,
        fontWeight: style.bold ? 700 : undefined,
        fontStyle: style.italic ? 'italic' : undefined,
        textDecoration: style.underline ? 'underline' : undefined,
        color: style.color,
        justifyContent: alignOf(style.align),
        alignItems: vAlignOf(style.valign),
      }}
      title={cell.f ? `${cell.f} → ${displayValue(cell)}` : undefined}
    >
      {highlight?.query ? markMatches(text, highlight.query, highlight.current) : text}
    </div>
  );
}

/** Wrap search matches in <mark> so Find shows where the hits are. */
function markMatches(text, query, current) {
  const source = String(text);
  const needle = String(query);
  if (!needle) return source;
  const parts = [];
  const lower = source.toLowerCase();
  const target = needle.toLowerCase();
  let i = 0;
  let key = 0;
  while (i < source.length) {
    const at = lower.indexOf(target, i);
    if (at === -1) {
      parts.push(source.slice(i));
      break;
    }
    if (at > i) parts.push(source.slice(i, at));
    parts.push(
      <mark key={key++} className={`findhit${current ? ' is-current' : ''}`}>
        {source.slice(at, at + needle.length)}
      </mark>
    );
    i = at + needle.length;
  }
  return parts;
}

function alignOf(align) {
  if (align === 'left') return 'flex-start';
  if (align === 'center') return 'center';
  if (align === 'right') return 'flex-end';
  return undefined; // fall back to the type-based class
}

/** Excel's default is bottom, which is why an unset cell sits on the baseline. */
function vAlignOf(valign) {
  if (valign === 'top') return 'flex-start';
  if (valign === 'middle') return 'center';
  if (valign === 'bottom') return 'flex-end';
  return undefined;
}

function cellVisualStyle(cell) {
  const style = cell?.style;
  if (!style) return undefined;
  const out = { ...borderStyles(style.border) };
  if (style.bg) out.background = style.bg;
  return out;
}

/**
 * In-cell editor. Enter/Tab commit with a direction so the selection advances the
 * way it does in Excel; Escape restores the previous content.
 *
 * A textarea rather than an input, because Alt+Enter has to put a real newline
 * in the cell — the way every two-line column heading in every workbook is
 * made. Enter still commits, so it behaves like a single-line box until asked
 * not to.
 */
function CellEditor({ initial, onCommit, onCancel }) {
  const ref = useRef(null);
  const [value, setValue] = useState(initial ?? '');

  useLayoutEffect(() => {
    const el = ref.current;
    el?.focus();
    el?.setSelectionRange(el.value.length, el.value.length);
  }, []);

  return (
    <textarea
      ref={ref}
      className="cell__editor"
      rows={1}
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' && e.altKey) {
          // 셀 안에서 줄 바꾸기.
          e.preventDefault();
          const el = e.currentTarget;
          const at = el.selectionStart;
          const next = `${value.slice(0, at)}\n${value.slice(el.selectionEnd)}`;
          setValue(next);
          requestAnimationFrame(() => ref.current?.setSelectionRange(at + 1, at + 1));
          return;
        }
        if (e.key === 'Enter') {
          e.preventDefault();
          onCommit(value, e.shiftKey ? 'up' : 'down');
          return;
        }
        if (e.key === 'Tab') {
          e.preventDefault();
          onCommit(value, e.shiftKey ? 'left' : 'right');
          return;
        }
        if (e.key === 'Escape') {
          e.preventDefault();
          onCancel();
          return;
        }
        // F4 cycles the $ locks on the reference under the caret, as in Excel.
        if (e.key === 'F4') {
          const next = cycleRefLocks(value, e.currentTarget.selectionStart);
          if (!next) return;
          e.preventDefault();
          setValue(next.value);
          requestAnimationFrame(() => ref.current?.setSelectionRange(next.caret, next.caret));
        }
      }}
      onBlur={() => onCommit(value, null)}
      spellCheck={false}
      wrap="off"
    />
  );
}

function maxUsedRow(cells) {
  let max = 0;
  for (const ref of Object.keys(cells)) {
    const m = ref.match(/(\d+)$/);
    if (m) max = Math.max(max, Number(m[1]));
  }
  return max;
}

function maxUsedCol(cells) {
  let max = 0;
  for (const ref of Object.keys(cells)) {
    const m = ref.match(/^([A-Z]+)/);
    if (!m) continue;
    let n = 0;
    for (const ch of m[1]) n = n * 26 + (ch.charCodeAt(0) - 64);
    max = Math.max(max, n);
  }
  return max;
}
