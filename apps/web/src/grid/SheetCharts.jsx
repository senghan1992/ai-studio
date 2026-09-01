import React, { useEffect, useRef, useState } from 'react';
import ChartView from '../components/ChartView.jsx';

const MIN_W = 200;
const MIN_H = 140;

/**
 * Charts floating over the grid, as Excel places them.
 *
 * They live inside the scrolling area so they travel with the cells, and they are
 * moved and resized by dragging rather than through a dialog. The chart itself is
 * a view of a cell range, so editing a number redraws it with no extra step.
 */
export default function SheetCharts({ sheet, zoom = 1, selectedId, onSelect, onMove, onEdit, onDelete }) {
  const [drag, setDrag] = useState(null);
  // Mirrored so the mouse-up handler can commit without doing work inside a
  // `setState` updater, which React may run more than once.
  const dragRef = useRef(null);
  dragRef.current = drag;
  const charts = sheet.charts ?? [];

  useEffect(() => {
    if (!drag) return undefined;
    const onMouseMove = (e) => {
      const d = dragRef.current;
      if (!d) return;
      const dx = (e.clientX - d.startX) / zoom;
      const dy = (e.clientY - d.startY) / zoom;
      const box =
        d.mode === 'move'
          ? { ...d.origin, x: Math.max(0, Math.round(d.origin.x + dx)), y: Math.max(0, Math.round(d.origin.y + dy)) }
          : {
              ...d.origin,
              w: Math.max(MIN_W, Math.round(d.origin.w + dx)),
              h: Math.max(MIN_H, Math.round(d.origin.h + dy)),
            };
      dragRef.current = { ...d, box };
      setDrag(dragRef.current);
    };
    const onMouseUp = () => {
      const d = dragRef.current;
      setDrag(null);
      dragRef.current = null;
      if (d?.box && changed(d.box, d.origin)) onMove(d.id, d.box);
    };
    const onKey = (e) => {
      if (e.key !== 'Escape') return;
      setDrag(null);
      dragRef.current = null;
    };
    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousemove', onMouseMove);
      window.removeEventListener('mouseup', onMouseUp);
      window.removeEventListener('keydown', onKey);
    };
  }, [drag, zoom, onMove]);

  if (!charts.length) return null;

  return (
    <>
      {charts.map((chart) => {
        const live = drag?.id === chart.id && drag.box ? { ...chart, ...drag.box } : chart;
        const selected = selectedId === chart.id;
        return (
          <div
            key={chart.id}
            className={`sheetchart${selected ? ' is-selected' : ''}`}
            style={{
              left: Math.round(live.x * zoom),
              top: Math.round(live.y * zoom),
              width: Math.round(live.w * zoom),
              height: Math.round(live.h * zoom),
            }}
            onMouseDown={(e) => {
              if (e.button !== 0) return;
              e.stopPropagation();
              onSelect(chart.id);
              dragRef.current = {
                id: chart.id,
                mode: 'move',
                startX: e.clientX,
                startY: e.clientY,
                origin: { x: chart.x, y: chart.y, w: chart.w, h: chart.h },
                box: null,
              };
              setDrag(dragRef.current);
            }}
            onDoubleClick={(e) => {
              e.stopPropagation();
              onEdit(chart.id);
            }}
            role="figure"
            aria-label={chart.spec?.title || '차트'}
          >
            <ChartView
              spec={chart.spec}
              sheet={sheet}
              width={live.w * zoom}
              height={live.h * zoom}
              surface="#ffffff"
            />

            {selected && (
              <>
                <div className="sheetchart__bar" onMouseDown={(e) => e.stopPropagation()}>
                  <button type="button" title="차트 편집" onClick={() => onEdit(chart.id)}>
                    ✎
                  </button>
                  <button type="button" title="차트 삭제" onClick={() => onDelete(chart.id)}>
                    ×
                  </button>
                </div>
                <div
                  className="sheetchart__grip"
                  title="크기 조절"
                  onMouseDown={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    dragRef.current = {
                      id: chart.id,
                      mode: 'resize',
                      startX: e.clientX,
                      startY: e.clientY,
                      origin: { x: chart.x, y: chart.y, w: chart.w, h: chart.h },
                      box: null,
                    };
                    setDrag(dragRef.current);
                  }}
                />
              </>
            )}
          </div>
        );
      })}
    </>
  );
}

function changed(a, b) {
  return a.x !== b.x || a.y !== b.y || a.w !== b.w || a.h !== b.h;
}
