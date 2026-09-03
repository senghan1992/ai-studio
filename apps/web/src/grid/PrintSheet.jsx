import React from 'react';
import { toRef, indexToCol, usedRange, displayValue, LIMITS } from '../core/index.js';
import { mergeCovering } from './gridOps.js';
import ChartView from '../components/ChartView.jsx';
import { borderStyles } from '../lib/borderStyle.js';

/** Rows beyond this print as a note rather than a thousand-page document. */
const PRINT_ROW_LIMIT = 1000;

/**
 * The active sheet's used range as plain paper, visible only when printing.
 *
 * Excel parity decides the look: no gridlines and no row/column headings by
 * default — only the borders, fills and merges the author actually set print.
 * A sheet wider than a portrait page turns the paper sideways, which is what
 * Excel's automatic orientation does with a wide print area.
 */
export default function PrintSheet({ sheet }) {
  const range = usedRange(sheet?.cells);
  if (!range) return null;

  const cols = range.maxCol + 1;
  const rows = Math.min(range.maxRow + 1, PRINT_ROW_LIMIT);
  const width = (c) => sheet.colWidths?.[indexToCol(c)] ?? LIMITS.colWidth;
  const totalWidth = Array.from({ length: cols }, (_, c) => width(c)).reduce((a, b) => a + b, 0);
  const orientation = totalWidth > 700 ? 'landscape' : 'portrait';

  return (
    <div className="printsheet" aria-hidden="true">
      <style>{`@media print { @page { size: A4 ${orientation}; margin: 12mm; } }`}</style>
      <table className="printsheet__table" style={{ fontSize: LIMITS.cellPx ?? 15 }}>
        <colgroup>
          {Array.from({ length: cols }, (_, c) => (
            <col key={c} style={{ width: width(c) }} />
          ))}
        </colgroup>
        <tbody>
          {Array.from({ length: rows }, (_, r) => (
            <tr key={r} style={{ height: sheet.rowHeights?.[String(r + 1)] }}>
              {Array.from({ length: cols }, (_, c) => {
                const merge = mergeCovering(sheet, r, c);
                if (merge && !(merge.r1 === r && merge.c1 === c)) return null;
                const cell = sheet.cells[toRef(c, r)];
                return (
                  <td
                    key={c}
                    rowSpan={merge ? merge.r2 - merge.r1 + 1 : undefined}
                    colSpan={merge ? merge.c2 - merge.c1 + 1 : undefined}
                    style={cellPrintStyle(cell)}
                  >
                    {cell ? displayValue(cell) : ''}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
      {range.maxRow + 1 > PRINT_ROW_LIMIT && (
        <p className="printsheet__note">
          … {range.maxRow + 1 - PRINT_ROW_LIMIT}개 행이 더 있습니다. 전체 데이터는 .xlsx 또는 .csv로
          내보내세요.
        </p>
      )}

      {/* Excel prints a sheet's charts; leaving them off the paper would be a
          louder difference than printing them below the cells instead of over
          them, which a flowing page cannot reproduce. */}
      {(sheet.charts ?? []).map((chart) => (
        <div key={chart.id} className="printsheet__chart" style={{ width: chart.w, height: chart.h }}>
          <ChartView spec={chart.spec} sheet={sheet} width={chart.w} height={chart.h} surface="#ffffff" />
        </div>
      ))}
    </div>
  );
}


/** The author's own formatting, and nothing the screen adds for editing. */
function cellPrintStyle(cell) {
  const style = cell?.style ?? {};
  const out = {
    textAlign: style.align ?? (cell?.t === 'n' || cell?.t === 'd' ? 'right' : 'left'),
    verticalAlign: style.valign === 'middle' ? 'middle' : style.valign === 'top' ? 'top' : 'bottom',
    whiteSpace: style.wrap ? 'pre-wrap' : 'nowrap',
  };
  if (style.fontSize) out.fontSize = `${style.fontSize}px`;
  if (style.bold) out.fontWeight = 700;
  if (style.italic) out.fontStyle = 'italic';
  if (style.underline) out.textDecoration = 'underline';
  if (style.color) out.color = style.color;
  if (style.bg) out.background = style.bg;
  Object.assign(out, borderStyles(style.border));
  return out;
}
