import React, { useMemo, useState } from 'react';
import { CHART_TYPES, CHART_TYPE_LABELS, normalizeChartSpec, resolveChartSpec } from '../core/index.js';
import { renderChartSvg } from '../lib/chartSvg.js';
import { Dialog, Field, Select } from './ui.jsx';

/**
 * Build or edit a chart, with a live preview.
 *
 * Two ways in, matching where charts come from: a spreadsheet chart points at a
 * cell range and stays live; a deck or document chart carries its own numbers,
 * typed as a small tab/comma-separated grid so pasting from anywhere works.
 */
export default function ChartDialog({ initial, sheet, rangeHint, onCancel, onConfirm }) {
  const start = normalizeChartSpec(initial ?? {});
  const [type, setType] = useState(start.type);
  const [title, setTitle] = useState(start.title);
  const [stacked, setStacked] = useState(start.options.stacked);
  const [valueLabels, setValueLabels] = useState(start.options.valueLabels);
  const [orientation, setOrientation] = useState(start.orientation);
  const [mode, setMode] = useState(start.range || (sheet && rangeHint) ? 'range' : 'table');
  const [range, setRange] = useState(start.range ?? rangeHint ?? '');
  const [table, setTable] = useState(() => specToTable(start));

  const spec = useMemo(() => {
    const base = {
      type,
      title,
      orientation,
      options: { stacked, valueLabels },
    };
    if (mode === 'range') return normalizeChartSpec({ ...base, range });
    return normalizeChartSpec({ ...base, ...tableToData(table) });
  }, [type, title, stacked, valueLabels, orientation, mode, range, table]);

  const resolved = useMemo(() => (sheet ? resolveChartSpec(spec, sheet) : spec), [spec, sheet]);
  const preview = useMemo(
    () => renderChartSvg(resolved, { width: 460, height: 260, surface: '#ffffff' }),
    [resolved]
  );

  const usable = resolved.series.length > 0;

  return (
    <Dialog
      title={initial ? '차트 편집' : '차트 삽입'}
      confirmLabel={initial ? '적용' : '삽입'}
      onCancel={onCancel}
      onConfirm={() => usable && onConfirm(spec)}
    >
      <div className="chartdlg">
        <div className="chartdlg__form">
          <Field label="차트 종류">
            <Select
              value={type}
              onChange={setType}
              options={CHART_TYPES.map((t) => ({ value: t, label: CHART_TYPE_LABELS[t] }))}
              title="차트 종류"
            />
          </Field>

          <Field label="제목">
            <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="분기별 매출" />
          </Field>

          {sheet && (
            <Field label="데이터 원본">
              <Select
                value={mode}
                onChange={setMode}
                options={[
                  { value: 'range', label: '셀 범위 (값이 바뀌면 차트도 바뀜)' },
                  { value: 'table', label: '직접 입력' },
                ]}
                title="데이터 원본"
              />
            </Field>
          )}

          {mode === 'range' ? (
            <>
              <Field label="범위 (첫 행은 계열 이름, 첫 열은 항목 이름)">
                <input value={range} onChange={(e) => setRange(e.target.value)} placeholder="A1:D5" />
              </Field>
              <Field label="계열 방향">
                <Select
                  value={orientation}
                  onChange={setOrientation}
                  options={[
                    { value: 'columns', label: '열이 계열' },
                    { value: 'rows', label: '행이 계열' },
                  ]}
                  title="계열 방향"
                />
              </Field>
            </>
          ) : (
            <Field label="데이터 (탭 또는 쉼표 구분, 첫 행은 계열 이름)">
              <textarea
                className="chartdlg__table"
                value={table}
                onChange={(e) => setTable(e.target.value)}
                rows={7}
                spellCheck={false}
                placeholder={'구분\t2025\t2026\n1분기\t98\t120\n2분기\t114\t138'}
              />
            </Field>
          )}

          <div className="chartdlg__row">
            {(type === 'column' || type === 'bar' || type === 'area') && (
              <label className="chartdlg__check">
                <input type="checkbox" checked={stacked} onChange={(e) => setStacked(e.target.checked)} />
                누적
              </label>
            )}
            <Field label="값 표시">
              <Select
                value={valueLabels}
                onChange={setValueLabels}
                options={[
                  { value: 'max', label: '최대값만' },
                  { value: 'ends', label: '양 끝만' },
                  { value: 'all', label: '전부' },
                  { value: 'none', label: '숨김' },
                ]}
                title="값 표시"
              />
            </Field>
          </div>

          {!usable && <p className="chartdlg__warn">표시할 데이터가 없습니다.</p>}
        </div>

        <div className="chartdlg__preview">
          <div className="chartdlg__previewlabel">미리보기</div>
          <div dangerouslySetInnerHTML={{ __html: preview }} />
          <p className="chartdlg__note">
            차트를 그리는 숫자는 <code>AI.md</code>에 표로도 저장됩니다 — AI가 그림이 아니라 값을 읽습니다.
          </p>
        </div>
      </div>
    </Dialog>
  );
}

/* ------------------------------------------------------------------ helpers */

function specToTable(spec) {
  if (!spec.series.length) return '구분\t계열1\n1월\t10\n2월\t20\n3월\t30';
  const header = ['구분', ...spec.series.map((s, i) => s.name || `계열${i + 1}`)];
  const rows = [header.join('\t')];
  const count = spec.labels.length || Math.max(...spec.series.map((s) => s.values.length));
  for (let i = 0; i < count; i++) {
    rows.push([spec.labels[i] ?? `${i + 1}`, ...spec.series.map((s) => s.values[i] ?? '')].join('\t'));
  }
  return rows.join('\n');
}

/** Parse the typed grid: first row names the series, first column the categories. */
function tableToData(text) {
  const rows = String(text ?? '')
    .split(/\r?\n/)
    .map((line) => line.split(/\t|,/).map((cell) => cell.trim()))
    .filter((cells) => cells.some((cell) => cell !== ''));
  if (rows.length < 2) return { labels: [], series: [] };

  const header = rows[0];
  const labels = rows.slice(1).map((r) => r[0] ?? '');
  const series = [];
  for (let c = 1; c < header.length; c++) {
    series.push({
      name: header[c] || `계열${c}`,
      values: rows.slice(1).map((r) => {
        const raw = (r[c] ?? '').replace(/,/g, '');
        if (raw === '') return null;
        const n = Number(raw);
        return Number.isFinite(n) ? n : null;
      }),
    });
  }
  return { labels, series };
}
