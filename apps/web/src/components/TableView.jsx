import React, { useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import { renderMarkdown } from '../lib/markdown.js';
import { tableEdit, tableMove } from '../core/index.js';

/**
 * A table, drawn from its markdown and its layout.
 *
 * The cells' text is the markdown table — that is where it lives on disk — and
 * everything about how it looks comes from the `table` spec beside it. Rendering
 * the two together is what makes an imported Word or PowerPoint table keep its
 * column widths, merges and banding instead of collapsing to a default grid.
 */
export default function TableView({
  md, spec, width, height,
  // Editing is opt-in: a table in the file inspector or a thumbnail is read-only.
  active, onChange, onActiveChange, onSelectBlock,
}) {
  const cells = useMemo(() => parseTable(md), [md]);
  const layout = spec ?? {};
  const columns = cells[0]?.length ?? 0;
  if (!columns) return null;

  const merges = useMemo(() => indexMerges(layout.merges, columns, cells.length), [
    layout.merges,
    columns,
    cells.length,
  ]);

  const editable = !!onChange;
  const at = active ?? null;

  const widths = colWidths(layout.cols, columns, width);
  const headerRow = layout.headerRow !== false;
  const banded = layout.bandedRows !== false;
  const style = layout.style ?? 'banded';

  /**
   * Commit a cell's text and move on, the way Office does.
   *
   * `Tab` from the last cell adds a row rather than trapping the cursor, which
   * is how a table gets longer while typing.
   */
  const commit = (col, row, text, move) => {
    let next = tableEdit(md, layout, 'setCell', { col, row, text });
    if (move === 'Tab' || move === 'Tab:back' || move === 'Enter') {
      const key = move === 'Enter' ? 'Enter' : 'Tab';
      const target = tableMove(next.md, next.table, col, row, key, move === 'Tab:back');
      if (target.addRow) {
        next = tableEdit(next.md, next.table, 'insert', { axis: 'row', row: next.rows });
        onChange(next.md, next.table);
        onActiveChange?.({ col: 0, row: next.rows - 1, editing: true });
        return;
      }
      onChange(next.md, next.table);
      onActiveChange?.({ col: target.col, row: target.row, editing: true });
      return;
    }
    onChange(next.md, next.table);
    onActiveChange?.({ col, row, editing: false });
  };

  /** Arrow keys walk cells when not typing in one. */
  useEffect(() => {
    if (!editable || !at || at.editing) return undefined;
    const onKey = (event) => {
      const tag = document.activeElement?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
      if (event.key === 'Tab') {
        event.preventDefault();
        const target = tableMove(md, layout, at.col, at.row, 'Tab', event.shiftKey);
        if (target.addRow) {
          const next = tableEdit(md, layout, 'insert', { axis: 'row', row: cells.length });
          onChange(next.md, next.table);
          onActiveChange?.({ col: 0, row: next.rows - 1, editing: true });
          return;
        }
        onActiveChange?.({ col: target.col, row: target.row, editing: false });
        return;
      }
      if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) {
        event.preventDefault();
        const target = tableMove(md, layout, at.col, at.row, event.key, false);
        onActiveChange?.({ col: target.col, row: target.row, editing: false });
        return;
      }
      if (event.key === 'Enter' || event.key === 'F2') {
        event.preventDefault();
        onActiveChange?.({ ...at, editing: true });
        return;
      }
      if (event.key === 'Delete' || event.key === 'Backspace') {
        event.preventDefault();
        const next = tableEdit(md, layout, 'setCell', { col: at.col, row: at.row, text: '' });
        onChange(next.md, next.table);
        return;
      }
      // Any printable key starts editing with that character, as in a spreadsheet.
      if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
        event.preventDefault();
        const next = tableEdit(md, layout, 'setCell', {
          col: at.col,
          row: at.row,
          text: event.key,
        });
        onChange(next.md, next.table);
        onActiveChange?.({ ...at, editing: true });
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [editable, at, md, layout, cells.length, onChange, onActiveChange]);

  return (
    <table
      className={`tableblock tableblock--${style}${editable ? ' is-editable' : ''}`}
      style={{ width: width ? `${width}px` : '100%', height: height ? `${height}px` : undefined }}
    >
      <colgroup>
        {widths.map((w, i) => (
          <col key={i} style={{ width: `${w}px` }} />
        ))}
      </colgroup>
      <tbody>
        {cells.map((row, r) => (
          <tr key={r} style={rowHeight(layout.rows, r)}>
            {row.map((text, c) => {
              const merge = merges.get(key(c, r));
              // A cell swallowed by a merge is not rendered at all; the anchor's
              // span covers it.
              if (merge && !merge.anchor) return null;

              const format = layout.cells?.[key(c, r)] ?? {};
              const isHeader = headerRow && r === 0;
              const Cell = isHeader ? 'th' : 'td';
              const isActive = editable && at?.col === c && at?.row === r;
              const bodyRow = headerRow ? r - 1 : r;
              const stripe = banded && style !== 'borderless' && bodyRow % 2 === 1;

              return (
                <Cell
                  key={c}
                  colSpan={merge?.cols > 1 ? merge.cols : undefined}
                  rowSpan={merge?.rows > 1 ? merge.rows : undefined}
                  className={[stripe ? 'is-banded' : '', isActive ? 'is-active' : '']
                    .filter(Boolean)
                    .join(' ') || undefined}
                  style={{
                    textAlign: format.align,
                    verticalAlign: valign(format.valign),
                    background: format.fill,
                    color: format.color,
                    fontWeight: layout.firstCol && c === 0 ? 700 : undefined,
                  }}
                  onPointerDown={
                    editable
                      ? (e) => {
                          // Clicking a cell selects the table *and* puts the
                          // cursor in the cell, as it does in Office. The
                          // pointer stops here so no move drag starts — the
                          // table is moved by the frame around it.
                          e.stopPropagation();
                          onSelectBlock?.();
                          onActiveChange?.({ col: c, row: r, editing: false });
                        }
                      : undefined
                  }
                  onDoubleClick={
                    editable
                      ? (e) => {
                          e.stopPropagation();
                          onSelectBlock?.();
                          onActiveChange?.({ col: c, row: r, editing: true });
                        }
                      : undefined
                  }
                >
                  {editable && at?.col === c && at?.row === r && at.editing ? (
                    <CellEditor
                      value={text}
                      onCommit={(next, move) => commit(c, r, next, move)}
                      onCancel={() => onActiveChange?.({ col: c, row: r, editing: false })}
                    />
                  ) : (
                    // The text is markdown, and the same sanitiser the rest of
                    // the editor uses runs over it.
                    <span
                      className="tableblock__text"
                      dangerouslySetInnerHTML={{ __html: renderMarkdown(text) }}
                    />
                  )}
                </Cell>
              );
            })}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/**
 * In-cell editor.
 *
 * A textarea rather than an input: a cell holds markdown and can be several
 * lines, and Office's tables take a line break inside a cell too. `Tab` and
 * `Enter` commit with a direction so the cursor advances the way it does there;
 * `Shift+Enter` inserts the line break instead.
 */
function CellEditor({ value, onCommit, onCancel }) {
  const ref = useRef(null);

  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    node.focus();
    // Select everything, so typing replaces — the spreadsheet convention people
    // already have from the Grid editor in this same app.
    node.select();
  }, []);

  return (
    <textarea
      ref={ref}
      className="tableblock__editor"
      defaultValue={value}
      rows={1}
      spellCheck={false}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault();
          onCancel();
          return;
        }
        if (event.key === 'Tab') {
          event.preventDefault();
          onCommit(event.currentTarget.value, event.shiftKey ? 'Tab:back' : 'Tab');
          return;
        }
        if (event.key === 'Enter' && !event.shiftKey) {
          event.preventDefault();
          onCommit(event.currentTarget.value, 'Enter');
        }
        // Shift+Enter falls through and inserts a newline.
      }}
      onBlur={(event) => onCommit(event.currentTarget.value, null)}
    />
  );
}

const key = (col, row) => `${colName(col)}${row + 1}`;

function colName(index) {
  let n = Math.max(0, index) + 1;
  let out = '';
  while (n > 0) {
    const rem = (n - 1) % 26;
    out = String.fromCharCode(65 + rem) + out;
    n = Math.floor((n - 1) / 26);
  }
  return out;
}

function colIndex(name) {
  let n = 0;
  for (const ch of name.toUpperCase()) {
    const v = ch.charCodeAt(0) - 65;
    if (v < 0 || v > 25) return -1;
    n = n * 26 + v + 1;
  }
  return n - 1;
}

/** Every cell a merge covers, mapped to its span and whether it is the anchor. */
function indexMerges(merges, columns, rows) {
  const out = new Map();
  for (const spec of merges ?? []) {
    const [from, to] = String(spec).split(':');
    const start = parseRef(from);
    const end = parseRef(to);
    if (!start || !end) continue;
    const c0 = Math.min(start.col, end.col);
    const c1 = Math.max(start.col, end.col);
    const r0 = Math.min(start.row, end.row);
    const r1 = Math.max(start.row, end.row);
    if (c1 >= columns || r1 >= rows) continue;
    for (let r = r0; r <= r1; r++) {
      for (let c = c0; c <= c1; c++) {
        out.set(key(c, r), {
          anchor: c === c0 && r === r0,
          cols: c1 - c0 + 1,
          rows: r1 - r0 + 1,
        });
      }
    }
  }
  return out;
}

function parseRef(text) {
  const m = /^\$?([A-Za-z]{1,3})\$?([1-9]\d{0,6})$/.exec(String(text ?? '').trim());
  if (!m) return null;
  const col = colIndex(m[1]);
  if (col < 0) return null;
  return { col, row: Number(m[2]) - 1 };
}

/** Column widths, sharing what is left over among the unassigned columns. */
function colWidths(cols, columns, total) {
  const assigned = (cols ?? []).slice(0, columns).map((w) => (w > 0 ? w : 0));
  const known = assigned.reduce((a, b) => a + b, 0);
  const unknown = columns - assigned.filter((w) => w > 0).length;
  const share = unknown > 0 ? Math.max(24, ((total || 600) - known) / unknown) : 0;
  return Array.from({ length: columns }, (_, i) =>
    assigned[i] > 0 ? assigned[i] : share || (total || 600) / columns
  );
}

function rowHeight(rows, index) {
  const h = rows?.[index];
  return h > 0 ? { height: `${h}px` } : undefined;
}

function valign(value) {
  if (value === 'middle') return 'middle';
  if (value === 'bottom') return 'bottom';
  if (value === 'top') return 'top';
  return undefined;
}

/**
 * The cells of a markdown table.
 *
 * Kept in step with `crates/ai-format/src/table.rs`: the alignment row is
 * dropped, `\|` is an escaped pipe, and ragged rows are padded so a hand-edited
 * table still draws as a rectangle.
 */
export function parseTable(md) {
  const rows = String(md ?? '')
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.startsWith('|') && !/^\|[\s:|-]*-[\s:|-]*\|?$/.test(line))
    .map((line) => splitCells(line.replace(/^\|/, '').replace(/\|$/, '')));

  const width = rows.reduce((max, row) => Math.max(max, row.length), 0);
  for (const row of rows) {
    while (row.length < width) row.push('');
  }
  return rows;
}

function splitCells(row) {
  const out = [];
  let cell = '';
  for (let i = 0; i < row.length; i++) {
    const ch = row[i];
    if (ch === '\\' && (row[i + 1] === '|' || row[i + 1] === '\\')) {
      cell += row[i + 1];
      i++;
    } else if (ch === '|') {
      out.push(cell.trim());
      cell = '';
    } else {
      cell += ch;
    }
  }
  out.push(cell.trim());
  return out;
}
