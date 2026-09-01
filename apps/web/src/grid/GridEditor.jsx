import React, { useCallback, useEffect, useMemo, useState } from 'react';
import {
  makeSheet, usedRange, newBlockId, toRef, editValue, FUNCTION_NAMES,
} from '../core/index.js';

import Shell from '../components/Shell.jsx';
import FileInspector from '../components/FileInspector.jsx';
import FileMenu from '../components/FileMenu.jsx';
import FindBar from '../components/FindBar.jsx';
import ChartDialog from '../components/ChartDialog.jsx';
import {
  Ribbon, Group, Btn, Select, ColorRow, Dialog, Field,
  ContextMenu, useContextMenu, ZoomSlider,
} from '../components/ui.jsx';
import SheetView from './Sheet.jsx';
import {
  setCellInput, patchCells, clearRange, structuralEdit,
  rangeToTsv, rangeToFormulaTsv, pasteTsv,
  normalizeRange, rangeRefs, rangeLabel, selectionStats, withRecalc,
  fillRange, mergeSelection, unmergeSelection, mergeCovering,
  applyBorders, BORDER_PRESETS, setColWidth, setRowHeight, autoFitColumn,
  resolveTarget, findCells, replaceInCells,
} from './gridOps.js';

const TABS = ['파일', '홈', '삽입', '수식', '데이터', 'AI'];

const FORMATS = [
  { value: '', label: '일반' },
  { value: '#,##0', label: '1,234' },
  { value: '#,##0.00', label: '1,234.00' },
  { value: '0%', label: '12%' },
  { value: '0.0%', label: '12.3%' },
  { value: '₩#,##0', label: '₩1,234' },
  { value: '$#,##0.00', label: '$1,234.00' },
  { value: 'yyyy-mm-dd', label: '2026-09-01' },
];

const FILLS = ['#f1f5f9', '#dbeafe', '#dcfce7', '#fee2e2', '#fef3c7', '#ede9fe'];
const TEXT_COLORS = ['#201f1e', '#107c41', '#c43e1c', '#185abd', '#7719aa', '#605e5c'];

const QUICK_FORMULAS = [
  { label: '합계', build: (r) => `=SUM(${r})` },
  { label: '평균', build: (r) => `=AVERAGE(${r})` },
  { label: '개수', build: (r) => `=COUNT(${r})` },
  { label: '최대', build: (r) => `=MAX(${r})` },
  { label: '최소', build: (r) => `=MIN(${r})` },
];

const FUNCTION_GROUPS = [
  { label: '수학', names: ['SUM', 'ROUND', 'ABS', 'SQRT', 'POWER', 'MOD', 'SUMPRODUCT'] },
  { label: '통계', names: ['AVERAGE', 'MEDIAN', 'MIN', 'MAX', 'COUNT', 'COUNTA', 'STDEV'] },
  { label: '논리', names: ['IF', 'IFS', 'IFERROR', 'AND', 'OR', 'NOT'] },
  { label: '텍스트', names: ['CONCAT', 'TEXTJOIN', 'LEFT', 'RIGHT', 'MID', 'LEN', 'TRIM', 'SUBSTITUTE', 'TEXT'] },
  { label: '찾기', names: ['VLOOKUP', 'HLOOKUP', 'INDEX', 'MATCH'] },
  { label: '날짜', names: ['TODAY', 'NOW', 'DATE', 'YEAR', 'MONTH', 'DAY', 'DAYS'] },
  { label: '조건부', names: ['SUMIF', 'COUNTIF', 'AVERAGEIF'] },
];

const emptyFind = { query: '', replacement: '', matchCase: false, inFormulas: false, at: null, replace: false };

export default function GridEditor({ ctl, onHome, notify, onNewProject }) {
  const { project, items: sheets, setItems, setTitle, save, saving, dirty, savedAt, undo, redo, canUndo, canRedo } = ctl;

  const [sheetIndex, setSheetIndex] = useState(0);
  const [sel, setSel] = useState({ row: 0, col: 0, row2: 0, col2: 0 });
  const [editing, setEditing] = useState(null);
  const [tab, setTab] = useState('홈');
  const [inspectorOpen, setInspectorOpen] = useState(true);
  const [showFormulas, setShowFormulas] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [nameDialog, setNameDialog] = useState(null);
  const [renameDialog, setRenameDialog] = useState(null);
  const [nameBox, setNameBox] = useState(null);
  const [find, setFind] = useState(null);
  const [chartDialog, setChartDialog] = useState(null);
  const [selectedChartId, setSelectedChartId] = useState(null);
  const ctx = useContextMenu();

  const index = Math.min(sheetIndex, Math.max(0, sheets.length - 1));
  const sheet = sheets[index];
  const range = normalizeRange(sel);

  const patchSheet = useCallback(
    (updater, options) => setItems((list) => list.map((s, i) => (i === index ? updater(s) : s)), options),
    [setItems, index]
  );

  /* ---------------------------------------------------------- cell editing */

  const activeRef = toRef(sel.col, sel.row);
  const activeCell = sheet?.cells?.[activeRef];

  const move = useCallback((direction, extend) => {
    setSel((current) => {
      const deltas = { up: [-1, 0], down: [1, 0], left: [0, -1], right: [0, 1] };
      const [dr, dc] = deltas[direction] ?? [0, 0];
      if (extend) {
        return {
          ...current,
          row2: Math.max(0, (current.row2 ?? current.row) + dr),
          col2: Math.max(0, (current.col2 ?? current.col) + dc),
        };
      }
      const row = Math.max(0, current.row + dr);
      const col = Math.max(0, current.col + dc);
      return { row, col, row2: row, col2: col };
    });
  }, []);

  const commitCell = useCallback(
    (value, direction) => {
      patchSheet((s) => setCellInput(s, sel.row, sel.col, value), { mergeKey: `cell:${activeRef}` });
      setEditing(null);
      if (direction) move(direction, false);
    },
    [patchSheet, sel.row, sel.col, activeRef, move]
  );

  /* -------------------------------------------------------------- commands */

  const copySelection = useCallback(
    async (formulas) => {
      const text = formulas ? rangeToFormulaTsv(sheet, range) : rangeToTsv(sheet, range);
      try {
        await navigator.clipboard.writeText(text);
        notify(`${rangeLabel(range)} 복사됨${formulas ? ' (수식)' : ''}`);
      } catch {
        notify('클립보드에 접근할 수 없습니다');
      }
      return text;
    },
    [sheet, range, notify]
  );

  const cutSelection = useCallback(async () => {
    await copySelection(false);
    patchSheet((s) => clearRange(s, rangeRefs(range), { keepStyle: true }));
  }, [copySelection, patchSheet, range]);

  const pasteFromClipboard = useCallback(async () => {
    try {
      const text = await navigator.clipboard.readText();
      if (!text) return;
      patchSheet((s) => pasteTsv(s, text, sel.row, sel.col));
      notify('붙여넣었습니다');
    } catch {
      notify('붙여넣기는 Ctrl+V를 눌러 주세요 (브라우저 권한)');
    }
  }, [patchSheet, sel.row, sel.col, notify]);

  const toggleStyle = useCallback(
    (key) => {
      const refs = rangeRefs(range);
      const allOn = refs.every((ref) => sheet.cells[ref]?.style?.[key]);
      patchSheet((s) => patchCells(s, refs, { style: { [key]: allOn ? null : true } }));
    },
    [range, sheet, patchSheet]
  );

  const applyFormat = useCallback((fmt) => patchSheet((s) => patchCells(s, rangeRefs(range), { fmt })), [patchSheet, range]);
  const applyStyle = useCallback((style) => patchSheet((s) => patchCells(s, rangeRefs(range), { style })), [patchSheet, range]);
  const setBorders = useCallback((preset) => patchSheet((s) => applyBorders(s, range, preset)), [patchSheet, range]);

  const merged = sheet ? mergeCovering(sheet, sel.row, sel.col) : null;
  const toggleMerge = useCallback(() => {
    if (merged) patchSheet((s) => unmergeSelection(s, range));
    else patchSheet((s) => mergeSelection(s, range));
  }, [merged, patchSheet, range]);

  const insertFormula = useCallback(
    (build) => {
      const target = suggestRange(sheet, sel.row, sel.col);
      const formula = build(target);
      patchSheet((s) => setCellInput(s, sel.row, sel.col, formula));
      notify(`${activeRef}에 ${formula} 입력`);
    },
    [sheet, sel.row, sel.col, patchSheet, activeRef, notify]
  );

  const doFill = useCallback((source, target) => patchSheet((s) => fillRange(s, source, target)), [patchSheet]);

  /* ---------------------------------------------------------------- charts */

  const insertChart = useCallback(
    (spec) => {
      const id = newBlockId();
      patchSheet((s) => ({
        ...s,
        charts: [
          ...(s.charts ?? []),
          // Drop it clear of the data so it does not land on top of the table.
          { id, x: 60, y: 60 + (s.charts?.length ?? 0) * 30, w: 480, h: 300, spec },
        ],
      }));
      setSelectedChartId(id);
      notify('차트를 추가했습니다');
    },
    [patchSheet, notify]
  );

  const updateChart = useCallback(
    (id, patch) =>
      patchSheet((s) => ({
        ...s,
        charts: (s.charts ?? []).map((c) => (c.id === id ? { ...c, ...patch } : c)),
      })),
    [patchSheet]
  );

  const deleteChart = useCallback(
    (id) => {
      patchSheet((s) => ({ ...s, charts: (s.charts ?? []).filter((c) => c.id !== id) }));
      setSelectedChartId(null);
    },
    [patchSheet]
  );

  const goTo = useCallback(
    (input) => {
      const target = resolveTarget(sheet, input);
      if (!target) {
        notify(`"${input}" 위치를 찾을 수 없습니다`);
        return false;
      }
      setSel(target);
      return true;
    },
    [sheet, notify]
  );

  /* ------------------------------------------------------------ find state */

  const findHits = useMemo(() => {
    if (!find?.query) return null;
    const hits = findCells(sheet, find.query, { matchCase: find.matchCase, inFormulas: find.inFormulas });
    hits.query = find.query;
    return hits;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sheet, find?.query, find?.matchCase, find?.inFormulas]);

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
    setSel({ row: hit.row, col: hit.col, row2: hit.row, col2: hit.col });
  }, [find?.query, find?.at, findHits]);

  const stepFind = useCallback(
    (delta) => {
      if (!findHits?.length) return;
      const next = ((find.at ?? -1) + delta + findHits.length) % findHits.length;
      const hit = findHits[next];
      setFind((f) => ({ ...f, at: next }));
      setSel({ row: hit.row, col: hit.col, row2: hit.row, col2: hit.col });
    },
    [findHits, find]
  );

  const replaceCurrent = useCallback(() => {
    const hit = findHits?.[find?.at ?? 0];
    if (!hit) return;
    patchSheet((s) => replaceInCells(s, [hit], find.query, find.replacement ?? '', { matchCase: find.matchCase }));
    notify(`${hit.ref} 바꿨습니다`);
  }, [findHits, find, patchSheet, notify]);

  const replaceAll = useCallback(() => {
    if (!findHits?.length) return;
    const count = findHits.length;
    patchSheet((s) => replaceInCells(s, findHits, find.query, find.replacement ?? '', { matchCase: find.matchCase }));
    notify(`${count}곳 바꿨습니다`);
    setFind((f) => ({ ...f, at: null }));
  }, [findHits, find, patchSheet, notify]);

  /* ------------------------------------------------------- keyboard driving */

  useEffect(() => {
    const onKey = (e) => {
      const mod = e.metaKey || e.ctrlKey;
      const tag = e.target?.tagName;
      const typing = tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';

      // Find/Replace open from anywhere, even with a field focused.
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
      if (editing || typing) return;

      const arrows = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right' };
      if (arrows[e.key]) {
        e.preventDefault();
        move(arrows[e.key], e.shiftKey);
        return;
      }
      if (e.key === 'Tab') {
        e.preventDefault();
        move(e.shiftKey ? 'left' : 'right', false);
        return;
      }
      if (e.key === 'Enter' || e.key === 'F2') {
        e.preventDefault();
        setEditing({ value: editValue(activeCell) });
        return;
      }
      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault();
        patchSheet((s) => clearRange(s, rangeRefs(range), { keepStyle: true }));
        return;
      }
      if (e.key === 'Home') {
        e.preventDefault();
        setSel(mod ? { row: 0, col: 0, row2: 0, col2: 0 } : { row: sel.row, col: 0, row2: sel.row, col2: 0 });
        return;
      }
      if (e.key === 'Escape' && find) {
        setFind(null);
        return;
      }
      if (mod && e.key.toLowerCase() === 'c') {
        e.preventDefault();
        copySelection(e.shiftKey);
        return;
      }
      if (mod && e.key.toLowerCase() === 'x') {
        e.preventDefault();
        cutSelection();
        return;
      }
      if (mod && 'biu'.includes(e.key.toLowerCase())) {
        e.preventDefault();
        toggleStyle({ b: 'bold', i: 'italic', u: 'underline' }[e.key.toLowerCase()]);
        return;
      }
      if (mod && e.key === '`') {
        e.preventDefault();
        setShowFormulas((v) => !v);
        return;
      }
      // A printable character starts an edit and replaces the cell, like Excel.
      if (!mod && !e.altKey && e.key.length === 1) {
        e.preventDefault();
        setEditing({ value: e.key });
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [editing, activeCell, range, sel.row, move, patchSheet, copySelection, cutSelection, toggleStyle, find]);

  // Native paste so Ctrl+V from Excel or a markdown table works.
  useEffect(() => {
    const onPaste = (e) => {
      if (editing) return;
      const tag = e.target?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA') return;
      const text = e.clipboardData?.getData('text/plain');
      if (!text) return;
      e.preventDefault();
      patchSheet((s) => pasteTsv(s, text, sel.row, sel.col));
      notify('붙여넣었습니다');
    };
    window.addEventListener('paste', onPaste);
    return () => window.removeEventListener('paste', onPaste);
  }, [editing, patchSheet, sel.row, sel.col, notify]);

  /* --------------------------------------------------------------- sheets */

  const addSheet = () => {
    setItems((list) => [...list, makeSheet({ name: uniqueSheetName(list) })]);
    setSheetIndex(sheets.length);
  };

  const deleteSheet = (at = index) => {
    if (sheets.length <= 1) {
      notify('마지막 시트는 삭제할 수 없습니다');
      return;
    }
    setItems((list) => list.filter((_, i) => i !== at));
    setSheetIndex(Math.max(0, at - 1));
  };

  const duplicateSheet = (at = index) => {
    setItems((list) => {
      const copy = { ...list[at], id: undefined, name: `${list[at].name} 사본` };
      return [...list.slice(0, at + 1), copy, ...list.slice(at + 1)];
    });
    setSheetIndex(at + 1);
  };

  const moveSheet = (from, to) => {
    if (to < 0 || to >= sheets.length || from === to) return;
    setItems((list) => {
      const next = [...list];
      const [moved] = next.splice(from, 1);
      next.splice(to, 0, moved);
      return next;
    });
    setSheetIndex(to);
  };

  const stats = useMemo(() => (sheet ? selectionStats(sheet, range) : null), [sheet, range]);
  const used = useMemo(() => (sheet ? usedRange(sheet.cells) : null), [sheet]);

  if (!sheet) return <div className="center-note">시트를 불러오는 중…</div>;

  /* -------------------------------------------------------- context menus */

  const singleCell = range.r1 === range.r2 && range.c1 === range.c2;

  const cellMenu = () => [
    { label: '잘라내기', shortcut: 'Ctrl+X', onClick: cutSelection },
    { label: '복사', shortcut: 'Ctrl+C', onClick: () => copySelection(false) },
    { label: '수식 복사', shortcut: 'Ctrl+Shift+C', onClick: () => copySelection(true) },
    { label: '붙여넣기', shortcut: 'Ctrl+V', onClick: pasteFromClipboard },
    '-',
    { label: '행 삽입', onClick: () => patchSheet((s) => structuralEdit(s, 'row', sel.row, 1)) },
    { label: '열 삽입', onClick: () => patchSheet((s) => structuralEdit(s, 'col', sel.col, 1)) },
    { label: '행 삭제', onClick: () => patchSheet((s) => structuralEdit(s, 'row', sel.row, -1)) },
    { label: '열 삭제', onClick: () => patchSheet((s) => structuralEdit(s, 'col', sel.col, -1)) },
    '-',
    { label: '이 범위로 차트 만들기', onClick: () => setChartDialog({ mode: 'insert', rangeHint: rangeLabel(range) }), disabled: singleCell },
    '-',
    { label: merged ? '병합 해제' : '셀 병합', onClick: toggleMerge, disabled: !merged && singleCell },
    { label: '테두리 (모두)', onClick: () => setBorders(BORDER_PRESETS.all) },
    { label: '테두리 지우기', onClick: () => setBorders(null) },
    '-',
    {
      label: '내용 지우기',
      shortcut: 'Delete',
      onClick: () => patchSheet((s) => clearRange(s, rangeRefs(range), { keepStyle: true })),
    },
    {
      label: '서식까지 지우기',
      danger: true,
      onClick: () => patchSheet((s) => clearRange(s, rangeRefs(range), { keepStyle: false })),
    },
  ];

  const colMenu = (c) => [
    { label: '왼쪽에 열 삽입', onClick: () => patchSheet((s) => structuralEdit(s, 'col', c, 1)) },
    { label: '열 삭제', onClick: () => patchSheet((s) => structuralEdit(s, 'col', c, -1)) },
    '-',
    { label: '너비 자동 맞춤', onClick: () => patchSheet((s) => setColWidth(s, c, autoFitColumn(s, c, used?.maxRow ?? 20))) },
    { label: '기본 너비로', onClick: () => patchSheet((s) => setColWidth(s, c, 0)) },
  ];

  const rowMenu = (r) => [
    { label: '위에 행 삽입', onClick: () => patchSheet((s) => structuralEdit(s, 'row', r, 1)) },
    { label: '행 삭제', onClick: () => patchSheet((s) => structuralEdit(s, 'row', r, -1)) },
    '-',
    { label: '기본 높이로', onClick: () => patchSheet((s) => setRowHeight(s, r, 0)) },
  ];

  const sheetTabMenu = (at) => [
    {
      label: '이름 변경',
      onClick: () => {
        setSheetIndex(at);
        setRenameDialog({ name: sheets[at].name });
      },
    },
    { label: '복제', onClick: () => duplicateSheet(at) },
    '-',
    { label: '왼쪽으로 이동', onClick: () => moveSheet(at, at - 1), disabled: at === 0 },
    { label: '오른쪽으로 이동', onClick: () => moveSheet(at, at + 1), disabled: at === sheets.length - 1 },
    '-',
    { label: '삭제', danger: true, disabled: sheets.length <= 1, onClick: () => deleteSheet(at) },
  ];

  /* --------------------------------------------------------------- ribbon */

  const ribbon = (
    <Ribbon tabs={TABS} active={tab} onTab={setTab}>
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
            <Btn icon="📋" label="붙여넣기" onClick={pasteFromClipboard} title="Ctrl+V" />
            <div className="rcol">
              <Btn small icon="⧉" label="복사" onClick={() => copySelection(false)} title="Ctrl+C" />
              <Btn small icon="✂" label="잘라내기" onClick={cutSelection} title="Ctrl+X" />
            </div>
          </Group>

          <Group label="글꼴">
            <div className="rcol">
              <div className="rrow">
                <Btn small icon="B" label="" title="굵게 (Ctrl+B)" pressed={!!activeCell?.style?.bold} onClick={() => toggleStyle('bold')} />
                <Btn small icon="I" label="" title="기울임 (Ctrl+I)" pressed={!!activeCell?.style?.italic} onClick={() => toggleStyle('italic')} />
                <Btn small icon="U" label="" title="밑줄 (Ctrl+U)" pressed={!!activeCell?.style?.underline} onClick={() => toggleStyle('underline')} />
              </div>
              <ColorRow colors={TEXT_COLORS} value={activeCell?.style?.color} onChange={(color) => applyStyle({ color })} title="글자 색" />
            </div>
          </Group>

          <Group label="채우기 · 테두리">
            <div className="rcol">
              <div className="rrow">
                <ColorRow colors={FILLS} value={activeCell?.style?.bg} onChange={(bg) => applyStyle({ bg })} title="채우기 색" />
                <Btn small icon="⌫" label="" title="채우기 지우기" onClick={() => applyStyle({ bg: null })} />
              </div>
              <div className="rrow">
                <Btn small icon="⊞" label="모두" title="모든 테두리" onClick={() => setBorders(BORDER_PRESETS.all)} />
                <Btn small icon="▢" label="외곽" title="외곽 테두리" onClick={() => setBorders('outline')} />
                <Btn small icon="▁" label="아래" title="아래쪽 테두리" onClick={() => setBorders(BORDER_PRESETS.bottom)} />
                <Btn small icon="⌧" label="없음" title="테두리 지우기" onClick={() => setBorders(null)} />
              </div>
            </div>
          </Group>

          <Group label="맞춤">
            <div className="rcol">
              <div className="rrow">
                {[
                  ['left', '⬅', '왼쪽'],
                  ['center', '↔', '가운데'],
                  ['right', '➡', '오른쪽'],
                ].map(([value, icon, title]) => (
                  <Btn
                    key={value}
                    small
                    icon={icon}
                    label=""
                    title={`${title} 맞춤`}
                    pressed={activeCell?.style?.align === value}
                    onClick={() => applyStyle({ align: value })}
                  />
                ))}
              </div>
              <Btn
                small
                icon="⿴"
                label={merged ? '병합 해제' : '셀 병합'}
                title="선택 영역을 하나로 병합"
                pressed={!!merged}
                onClick={toggleMerge}
              />
            </div>
          </Group>

          <Group label="표시 형식">
            <div className="rcol">
              <Select value={activeCell?.fmt ?? ''} onChange={applyFormat} options={FORMATS} title="표시 형식" width={128} />
              <div className="rrow">
                <Btn small label="1,000" title="천 단위 구분" onClick={() => applyFormat('#,##0')} />
                <Btn small label="%" title="백분율" onClick={() => applyFormat('0.0%')} />
                <Btn small label="₩" title="통화" onClick={() => applyFormat('₩#,##0')} />
              </div>
            </div>
          </Group>

          <Group label="셀">
            <div className="rcol">
              <div className="rrow">
                <Btn small icon="+" label="행" title="행 삽입" onClick={() => patchSheet((s) => structuralEdit(s, 'row', sel.row, 1))} />
                <Btn small icon="−" label="행" title="행 삭제" onClick={() => patchSheet((s) => structuralEdit(s, 'row', sel.row, -1))} />
              </div>
              <div className="rrow">
                <Btn small icon="+" label="열" title="열 삽입" onClick={() => patchSheet((s) => structuralEdit(s, 'col', sel.col, 1))} />
                <Btn small icon="−" label="열" title="열 삭제" onClick={() => patchSheet((s) => structuralEdit(s, 'col', sel.col, -1))} />
              </div>
            </div>
          </Group>

          <Group label="편집">
            <Btn icon="Σ" label="자동 합계" onClick={() => insertFormula(QUICK_FORMULAS[0].build)} />
            <Btn icon="🔍" label="찾기" title="Ctrl+F" onClick={() => setFind((f) => f ?? { ...emptyFind })} />
            <Btn
              icon="⌦"
              label="지우기"
              title="Delete"
              onClick={() => patchSheet((s) => clearRange(s, rangeRefs(range), { keepStyle: true }))}
            />
          </Group>
        </>
      )}

      {tab === '삽입' && (
        <>
          <Group label="빠른 계산">
            {QUICK_FORMULAS.map((f) => (
              <Btn key={f.label} icon="Σ" label={f.label} onClick={() => insertFormula(f.build)} />
            ))}
          </Group>
          <Group label="차트">
            <Btn
              icon="📊"
              label="차트"
              title="선택한 범위로 차트를 만듭니다"
              onClick={() => setChartDialog({ mode: 'insert', rangeHint: rangeLabel(range) })}
            />
            <Btn
              icon="✎"
              label="차트 편집"
              disabled={!selectedChartId}
              onClick={() => setChartDialog({ mode: 'edit', id: selectedChartId })}
            />
            <Btn
              icon="🗑"
              label="차트 삭제"
              disabled={!selectedChartId}
              onClick={() => deleteChart(selectedChartId)}
            />
          </Group>
          <Group label="이름">
            <Btn
              icon="🏷"
              label="범위 이름"
              title="선택 범위에 이름을 붙여 수식에서 사용합니다"
              onClick={() => setNameDialog({ name: '', target: rangeLabel(range) })}
            />
          </Group>
          <Group label="시트">
            <Btn icon="🞤" label="시트 추가" onClick={addSheet} />
            <Btn icon="⧉" label="시트 복제" onClick={() => duplicateSheet()} />
            <Btn icon="✎" label="이름 변경" onClick={() => setRenameDialog({ name: sheet.name })} />
            <Btn icon="🗑" label="시트 삭제" onClick={() => deleteSheet()} disabled={sheets.length <= 1} />
          </Group>
        </>
      )}

      {tab === '수식' && (
        <>
          {FUNCTION_GROUPS.map((group) => (
            <Group key={group.label} label={group.label}>
              <Select
                value=""
                onChange={(name) => name && setEditing({ value: `=${name}(` })}
                options={[{ value: '', label: '선택…' }, ...group.names.map((n) => ({ value: n, label: n }))]}
                title={`${group.label} 함수`}
                width={104}
              />
            </Group>
          ))}
          <Group label="검사">
            <Btn
              icon="ƒ"
              label={showFormulas ? '값 보기' : '수식 보기'}
              title="Ctrl+`"
              pressed={showFormulas}
              onClick={() => setShowFormulas((v) => !v)}
            />
            <Btn icon="↻" label="다시 계산" onClick={() => patchSheet((s) => withRecalc(s))} />
          </Group>
        </>
      )}

      {tab === '데이터' && (
        <>
          <Group label="정렬">
            <Btn icon="↑" label="오름차순" title="선택 범위를 첫 열 기준으로 정렬" onClick={() => patchSheet((s) => sortRange(s, range, true))} />
            <Btn icon="↓" label="내림차순" onClick={() => patchSheet((s) => sortRange(s, range, false))} />
          </Group>
          <Group label="틀 고정">
            <Select
              value={`${sheet.frozen?.rows ?? 0}:${sheet.frozen?.cols ?? 0}`}
              onChange={(v) => {
                const [rows, cols] = v.split(':').map(Number);
                patchSheet((s) => ({ ...s, frozen: { rows, cols } }));
              }}
              options={[
                { value: '0:0', label: '고정 없음' },
                { value: '1:0', label: '첫 행' },
                { value: '0:1', label: '첫 열' },
                { value: '1:1', label: '첫 행 + 첫 열' },
                { value: '2:1', label: '두 행 + 첫 열' },
              ]}
              title="틀 고정"
              width={140}
            />
          </Group>
          <Group label="찾기">
            <Btn icon="🔍" label="찾기" title="Ctrl+F" onClick={() => setFind((f) => f ?? { ...emptyFind })} />
            <Btn icon="⇄" label="바꾸기" title="Ctrl+H" onClick={() => setFind((f) => ({ ...(f ?? emptyFind), replace: true }))} />
          </Group>
          <Group label="범위">
            <div style={{ fontSize: 11.5, color: 'var(--ink-2)', padding: '2px 6px', lineHeight: 1.6 }}>
              {used ? (
                <>
                  데이터 범위 <code>A1:{toRef(used.maxCol, used.maxRow)}</code>
                  <br />
                  수식 {Object.values(sheet.cells).filter((c) => c.f).length}개 · 병합 {(sheet.merges ?? []).length}개
                </>
              ) : (
                '빈 시트'
              )}
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
          <Group label="AI가 읽는 방식">
            <div style={{ fontSize: 11.5, color: 'var(--ink-2)', maxWidth: 480, lineHeight: 1.6, padding: '2px 6px' }}>
              <code>.cells.json</code>에 수식과 값이, <code>.md</code>에 열 머리글(A·B·C)과 행 번호가 붙은
              표와 <strong>수식 목록</strong>, <strong>열 요약</strong>이 저장됩니다. AI는 <q>D4의 합계는?</q>{' '}
              같은 질문에 셀 주소로 답할 수 있습니다.
            </div>
          </Group>
        </>
      )}
    </Ribbon>
  );

  /* ---------------------------------------------------------------- render */

  return (
    <Shell
      type="grid"
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
            선택 <code>{rangeLabel(range)}</code>
          </span>
          {stats && stats.count > 0 && (
            <>
              <span>개수 {stats.count}</span>
              {stats.numeric > 0 && (
                <>
                  <span>합계 {formatStat(stats.sum)}</span>
                  <span>평균 {formatStat(stats.avg)}</span>
                </>
              )}
            </>
          )}
          {showFormulas && <span>수식 보기</span>}
          {(sheet.charts?.length ?? 0) > 0 && <span>차트 {sheet.charts.length}개</span>}
          <span className="statusbar__spacer" />
          <ZoomSlider value={zoom} onChange={setZoom} min={0.5} max={2} />
        </>
      }
    >
      <div className="gridwrap" style={{ flex: 1, minWidth: 0, position: 'relative' }}>
        <div className="formulabar">
          <input
            className="formulabar__ref"
            value={nameBox ?? rangeLabel(range)}
            onChange={(e) => setNameBox(e.target.value)}
            onFocus={(e) => e.target.select()}
            onBlur={() => setNameBox(null)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                if (goTo(e.currentTarget.value)) setNameBox(null);
                e.currentTarget.blur();
              } else if (e.key === 'Escape') {
                setNameBox(null);
                e.currentTarget.blur();
              }
            }}
            title="이름 상자 — 셀 주소나 범위 이름을 입력해 이동합니다"
            aria-label="이름 상자"
          />
          <span className="formulabar__fx" aria-hidden="true">
            fx
          </span>
          <FormulaInput
            key={`${activeRef}:${editing ? 'edit' : 'view'}`}
            value={editing?.value ?? editValue(activeCell)}
            onCommit={(value) => commitCell(value, 'down')}
          />
        </div>

        {find && (
          <FindBar
            state={find}
            hits={findHits}
            onChange={(patch) =>
              setFind((f) => ({ ...f, ...patch, at: patch.query !== undefined ? null : f.at }))
            }
            onNext={() => stepFind(1)}
            onPrev={() => stepFind(-1)}
            onReplace={replaceCurrent}
            onReplaceAll={replaceAll}
            onClose={() => setFind(null)}
          />
        )}

        <SheetView
          sheet={sheet}
          sel={sel}
          onSelChange={setSel}
          editing={editing}
          showFormulas={showFormulas}
          zoom={zoom}
          findHits={findHits}
          currentHit={findHits?.[find?.at ?? -1]?.ref}
          onEditStart={(row, col) => {
            setSel({ row, col, row2: row, col2: col });
            setEditing({ value: editValue(sheet.cells[toRef(col, row)]) });
          }}
          onCommit={commitCell}
          onEditCancel={() => setEditing(null)}
          onFill={doFill}
          onResizeCol={(c, width) => patchSheet((s) => setColWidth(s, c, width))}
          onResizeRow={(r, height) => patchSheet((s) => setRowHeight(s, r, height))}
          onAutoFitCol={(c) => patchSheet((s) => setColWidth(s, c, autoFitColumn(s, c, used?.maxRow ?? 20)))}
          onContextMenu={(e, info) =>
            ctx.open(
              e,
              info.kind === 'col' ? colMenu(info.index) : info.kind === 'row' ? rowMenu(info.index) : cellMenu()
            )
          }
          selectedChartId={selectedChartId}
          onSelectChart={setSelectedChartId}
          onMoveChart={(id, box) => updateChart(id, box)}
          onEditChart={(id) => setChartDialog({ mode: 'edit', id })}
          onDeleteChart={deleteChart}
        />

        <div className="sheettabs">
          {sheets.map((s, i) => (
            <button
              key={s.id ?? i}
              className="sheettab"
              aria-current={i === index}
              onClick={() => setSheetIndex(i)}
              onDoubleClick={() => {
                setSheetIndex(i);
                setRenameDialog({ name: s.name });
              }}
              onContextMenu={(e) => ctx.open(e, sheetTabMenu(i))}
              title={`${s.name} — 두 번 누르면 이름 변경, 우클릭으로 메뉴`}
            >
              {s.name}
            </button>
          ))}
          <button className="sheettab__add" onClick={addSheet} title="시트 추가" aria-label="시트 추가">
            +
          </button>
        </div>
      </div>

      {inspectorOpen && <FileInspector project={project} activeIndex={index} />}

      {ctx.menu && <ContextMenu {...ctx.menu} onClose={ctx.close} />}

      {nameDialog && (
        <Dialog
          title="범위 이름 지정"
          confirmLabel="지정"
          onCancel={() => setNameDialog(null)}
          onConfirm={() => {
            const name = nameDialog.name.trim();
            if (!name) return;
            patchSheet((s) => ({ ...s, names: { ...s.names, [name]: nameDialog.target } }));
            setNameDialog(null);
            notify(`${name} → ${nameDialog.target}`);
          }}
        >
          <p>
            수식에서 <code>=SUM({nameDialog.name || '이름'})</code> 처럼 쓸 수 있고, 이름 상자에 입력해
            이동할 수도 있습니다. <code>.md</code>의 <q>이름 있는 범위</q> 절에도 기록되어 AI가 의미를
            읽습니다.
          </p>
          <Field label={`이름 (범위 ${nameDialog.target})`}>
            <input
              value={nameDialog.name}
              onChange={(e) => setNameDialog({ ...nameDialog, name: e.target.value })}
              placeholder="매출데이터"
            />
          </Field>
        </Dialog>
      )}

      {chartDialog && (
        <ChartDialog
          initial={
            chartDialog.mode === 'edit'
              ? (sheet.charts ?? []).find((c) => c.id === chartDialog.id)?.spec
              : null
          }
          sheet={sheet}
          rangeHint={chartDialog.rangeHint}
          onCancel={() => setChartDialog(null)}
          onConfirm={(spec) => {
            if (chartDialog.mode === 'edit') updateChart(chartDialog.id, { spec });
            else insertChart(spec);
            setChartDialog(null);
          }}
        />
      )}

      {renameDialog && (
        <Dialog
          title="시트 이름 변경"
          confirmLabel="변경"
          onCancel={() => setRenameDialog(null)}
          onConfirm={() => {
            const name = renameDialog.name.trim();
            if (!name) return;
            patchSheet((s) => ({ ...s, name }));
            setRenameDialog(null);
          }}
        >
          <p>파일 이름도 함께 바뀝니다. 다음 저장 시 이전 파일은 정리됩니다.</p>
          <Field label="시트 이름">
            <input value={renameDialog.name} onChange={(e) => setRenameDialog({ ...renameDialog, name: e.target.value })} />
          </Field>
        </Dialog>
      )}
    </Shell>
  );
}

/* ------------------------------------------------------------ formula bar */

function FormulaInput({ value, onCommit }) {
  const [draft, setDraft] = useState(value ?? '');
  const [hint, setHint] = useState(null);

  useEffect(() => setDraft(value ?? ''), [value]);

  return (
    <div style={{ flex: 1, position: 'relative' }}>
      <input
        className="formulabar__input"
        value={draft}
        onChange={(e) => {
          setDraft(e.target.value);
          setHint(suggestFunction(e.target.value));
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            onCommit(draft);
            setHint(null);
          } else if (e.key === 'Escape') {
            e.preventDefault();
            setDraft(value ?? '');
            setHint(null);
            e.currentTarget.blur();
          }
        }}
        onBlur={() => setHint(null)}
        placeholder="값 또는 =수식"
        aria-label="수식 입력"
        spellCheck={false}
      />
      {hint && <div className="fxhint">{hint}</div>}
    </div>
  );
}

function suggestFunction(text) {
  const m = String(text).match(/([A-Za-z]{2,})$/);
  if (!m || !text.startsWith('=')) return null;
  const prefix = m[1].toUpperCase();
  const matches = FUNCTION_NAMES.filter((n) => n.startsWith(prefix)).slice(0, 6);
  return matches.length ? matches.join('  ') : null;
}

/* ------------------------------------------------------------------ utils */

/** The contiguous run of cells above (or left of) the target, for Σ buttons. */
function suggestRange(sheet, row, col) {
  let top = row - 1;
  while (top >= 0 && sheet.cells[toRef(col, top)] !== undefined) top--;
  const start = top + 1;
  if (start <= row - 1) return `${toRef(col, start)}:${toRef(col, row - 1)}`;

  let left = col - 1;
  while (left >= 0 && sheet.cells[toRef(left, row)] !== undefined) left--;
  const cstart = left + 1;
  if (cstart <= col - 1) return `${toRef(cstart, row)}:${toRef(col - 1, row)}`;

  return toRef(col, Math.max(0, row - 1));
}

/** Sort the selected block by its first column, carrying whole rows along. */
function sortRange(sheet, range, ascending) {
  const rows = [];
  for (let r = range.r1; r <= range.r2; r++) {
    const cells = [];
    for (let c = range.c1; c <= range.c2; c++) cells.push(sheet.cells[toRef(c, r)] ?? null);
    rows.push(cells);
  }

  const key = (cells) => {
    const v = cells[0]?.v;
    return v === null || v === undefined ? '' : v;
  };
  rows.sort((a, b) => {
    const av = key(a);
    const bv = key(b);
    const numeric = typeof av === 'number' && typeof bv === 'number';
    const cmp = numeric ? av - bv : String(av).localeCompare(String(bv), 'ko');
    return ascending ? cmp : -cmp;
  });

  const cells = { ...sheet.cells };
  rows.forEach((rowCells, i) => {
    rowCells.forEach((cell, j) => {
      const ref = toRef(range.c1 + j, range.r1 + i);
      if (cell) cells[ref] = cell;
      else delete cells[ref];
    });
  });
  return withRecalc({ ...sheet, cells });
}

function uniqueSheetName(list) {
  const names = new Set(list.map((s) => s.name));
  for (let i = list.length + 1; i < 200; i++) {
    const candidate = `시트${i}`;
    if (!names.has(candidate)) return candidate;
  }
  return `시트${Date.now()}`;
}

function formatStat(n) {
  if (n === null || n === undefined) return '—';
  const rounded = Math.round(n * 100) / 100;
  return rounded.toLocaleString('ko-KR');
}
