import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import {
  makeSection, newBlockId, PAGE_SIZES, headingLevel, plainText, countWords,
  serializeChartBlock, parseChartBlock,
} from '../core/index.js';

import Shell from '../components/Shell.jsx';
import FileInspector from '../components/FileInspector.jsx';
import FileMenu from '../components/FileMenu.jsx';
import FindBar from '../components/FindBar.jsx';
import ChartDialog from '../components/ChartDialog.jsx';
import ImageDialog from '../components/ImageDialog.jsx';
import ChartView from '../components/ChartView.jsx';
import {
  Ribbon, Group, Btn, Select, NumInput, ColorRow, Dialog, Field,
  ContextMenu, useContextMenu, ZoomSlider,
} from '../components/ui.jsx';
import { renderMarkdown, toggleWrap, toggleLinePrefix, continueList } from '../lib/markdown.js';
import { assetUrl, isProjectAsset } from '../api.js';
import { paginate, isPageBreak, PAGE_BREAK, contentHeightOf, contentWidthOf } from './paginate.js';

const TABS = ['파일', '홈', '삽입', '레이아웃', '보기', 'AI'];
const TEXT_COLORS = ['#201f1e', '#185abd', '#c43e1c', '#107c41', '#7719aa', '#605e5c'];
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
  const [imageDialog, setImageDialog] = useState(null);
  const [chartDialog, setChartDialog] = useState(null);
  const [focusRequest, setFocusRequest] = useState(null);
  const [find, setFind] = useState(null);
  const [heights, setHeights] = useState({});
  const ctx = useContextMenu();

  const index = Math.min(sectionIndex, Math.max(0, sections.length - 1));
  const section = sections[index];

  /* ------------------------------------------------------------ mutations */

  const patchSection = useCallback(
    (updater, options) => setItems((list) => list.map((s, i) => (i === index ? updater(s) : s)), options),
    [setItems, index]
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
        if (classify(md) !== 'chart' && classify(md) !== 'image') {
          setEditingId(id);
          setFocusRequest({ id, caret: 0 });
        }
      }
      return id;
    },
    [patchSection]
  );

  const splitBlock = useCallback(
    (blockId, caret) => {
      const newId = newBlockId();
      patchSection((s) => {
        const at = s.blocks.findIndex((b) => b.id === blockId);
        if (at < 0) return s;
        const block = s.blocks[at];
        const head = String(block.md ?? '').slice(0, caret);
        const tail = String(block.md ?? '').slice(caret);
        const next = [...s.blocks];
        next[at] = { ...block, md: head, type: classify(head) };
        next.splice(at + 1, 0, { id: newId, md: tail, type: classify(tail), override: block.override ?? null });
        return { ...s, blocks: next };
      });
      setSelectedId(newId);
      setEditingId(newId);
      setFocusRequest({ id: newId, caret: 0 });
    },
    [patchSection]
  );

  const mergeBackward = useCallback(
    (blockId) => {
      let target = null;
      let caret = 0;
      patchSection((s) => {
        const at = s.blocks.findIndex((b) => b.id === blockId);
        if (at <= 0) return s;
        const prev = s.blocks[at - 1];
        const block = s.blocks[at];
        target = prev.id;
        caret = String(prev.md ?? '').length;
        const next = [...s.blocks];
        next[at - 1] = { ...prev, md: `${prev.md ?? ''}${block.md ?? ''}` };
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
      if (e.key === 'Escape' && find) setFind(null);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [find, insertBlock, selectedId, notify]);

  /* ------------------------------------------------------------ pagination */

  const page = section?.page ?? { size: 'A4', margin: { top: 72, right: 72, bottom: 72, left: 72 } };
  const pageSize = PAGE_SIZES[page.size] ?? PAGE_SIZES.A4;
  const contentHeight = contentHeightOf(pageSize, page.margin);
  const contentWidth = contentWidthOf(pageSize, page.margin);

  const pages = useMemo(
    () => paginate(section?.blocks ?? [], heights, contentHeight),
    [section?.blocks, heights, contentHeight]
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
    ...(block.override ? [{ label: '서식 지우기', onClick: () => clearOverride(block.id) }] : []),
    '-',
    { label: '삭제', danger: true, disabled: section.blocks.length <= 1, onClick: () => deleteBlock(block.id) },
  ];

  /* ---------------------------------------------------------------- ribbon */

  const ribbon = (
    <Ribbon tabs={TABS} active={tab} onTab={setTab}>
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
                ]}
                title="문단 스타일"
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
                <Btn small icon="B" label="" title="굵게 (**)" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleWrap(md, a, b, '**'))} />
                <Btn small icon="I" label="" title="기울임 (*)" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleWrap(md, a, b, '*'))} />
                <Btn small icon="S" label="" title="취소선 (~~)" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleWrap(md, a, b, '~~'))} />
                <Btn small icon="`" label="" title="인라인 코드" disabled={!selected} onClick={() => transformMd((md, a, b) => toggleWrap(md, a, b, '`'))} />
              </div>
              <div className="rrow">
                <NumInput
                  value={selected?.override?.style?.fontSize ?? 15}
                  onChange={(v) => selected && patchOverride(selected.id, { style: { fontSize: v } })}
                  title="글자 크기"
                  min={8}
                  max={72}
                />
                <ColorRow
                  colors={TEXT_COLORS}
                  value={selected?.override?.style?.color}
                  onChange={(c) => selected && patchOverride(selected.id, { style: { color: c } })}
                  title="글자 색"
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
                    title={`${title} 정렬`}
                    disabled={!selected}
                    pressed={(selected?.override?.align ?? 'left') === value}
                    onClick={() => patchOverride(selected.id, { align: value })}
                  />
                ))}
                <Btn small icon="⇥" label="" title="들여쓰기 늘리기" disabled={!selected} onClick={() => patchOverride(selected.id, { indent: (selected.override?.indent ?? 0) + 24 })} />
                <Btn small icon="⇤" label="" title="들여쓰기 줄이기" disabled={!selected} onClick={() => patchOverride(selected.id, { indent: Math.max(0, (selected.override?.indent ?? 0) - 24) })} />
              </div>
            </div>
          </Group>

          <Group label="편집">
            <Btn icon="🔍" label="찾기" title="Ctrl+F" onClick={() => setFind((f) => f ?? { ...emptyFind })} />
            <Btn icon="⇄" label="바꾸기" title="Ctrl+H" onClick={() => setFind((f) => ({ ...(f ?? emptyFind), replace: true }))} />
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
            <Btn icon="▦" label="표" onClick={() => setTableDialog({ rows: 3, cols: 3 })} />
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
          <Group label="섹션">
            <Btn icon="🞤" label="섹션 추가" onClick={addSection} />
            <Btn icon="🗑" label="섹션 삭제" onClick={deleteSection} disabled={sections.length <= 1} />
          </Group>
        </>
      )}

      {tab === '레이아웃' && (
        <>
          <Group label="용지">
            <Select
              value={page.size}
              onChange={(size) => patchSection((s) => ({ ...s, page: { ...s.page, size } }))}
              options={Object.keys(PAGE_SIZES).map((k) => ({ value: k, label: k }))}
              title="용지 크기"
              width={92}
            />
          </Group>
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
    folder: project.folder,
    selected: selectedId === block.id,
    editing: editingId === block.id,
    focusRequest: focusRequest?.id === block.id ? focusRequest : null,
    onFocusHandled: () => setFocusRequest(null),
    onSelect: () => {
      setSelectedId(block.id);
      if (!openBlock(block)) setEditingId(block.id);
    },
    onChange: (md) => setBlockMd(block.id, md),
    onSplit: (caret) => splitBlock(block.id, caret),
    onMergeBackward: () => mergeBackward(block.id),
    onExit: () => setEditingId(null),
    onContextMenu: (e) => {
      setSelectedId(block.id);
      ctx.open(e, blockMenu(block));
    },
    highlight: find?.query ?? '',
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
        <div className="measure-layer md md--doc" style={{ width: contentWidth * zoom, fontSize: `${15 * zoom}px` }}>
          {section.blocks.filter((b) => !isPageBreak(b)).map((block) => (
            <div
              key={block.id}
              ref={(node) => {
                if (node) measureRefs.current.set(block.id, node);
                else measureRefs.current.delete(block.id);
              }}
            >
              <BlockContent block={block} folder={project.folder} width={contentWidth * zoom} />
            </div>
          ))}
        </div>
      </div>

      {inspectorOpen && <FileInspector project={project} activeIndex={index} />}

      {ctx.menu && <ContextMenu {...ctx.menu} onClose={ctx.close} />}

      {tableDialog && (
        <Dialog
          title="표 삽입"
          confirmLabel="삽입"
          onCancel={() => setTableDialog(null)}
          onConfirm={() => {
            insertBlock(selectedId, makeTable(tableDialog.rows, tableDialog.cols));
            setTableDialog(null);
          }}
        >
          <p>마크다운 표로 삽입됩니다. AI는 이 표를 그대로 읽을 수 있습니다.</p>
          <Field label="행 수 (머리글 제외)">
            <input type="number" min="1" max="20" value={tableDialog.rows} onChange={(e) => setTableDialog({ ...tableDialog, rows: Number(e.target.value) })} />
          </Field>
          <Field label="열 수">
            <input type="number" min="1" max="10" value={tableDialog.cols} onChange={(e) => setTableDialog({ ...tableDialog, cols: Number(e.target.value) })} />
          </Field>
        </Dialog>
      )}

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
}) {
  const ref = useRef(null);
  const override = block.override ?? {};

  const style = {
    textAlign: override.align,
    marginLeft: override.indent ? `${override.indent}px` : undefined,
    marginTop: override.spacing?.before ? `${override.spacing.before}px` : undefined,
    marginBottom: override.spacing?.after ? `${override.spacing.after}px` : undefined,
    color: override.style?.color,
    fontSize: override.style?.fontSize ? `${override.style.fontSize}px` : undefined,
    fontStyle: override.style?.italic ? 'italic' : undefined,
    fontWeight: override.style?.bold ? 700 : undefined,
    textDecoration: override.style?.underline ? 'underline' : undefined,
    background: override.style?.bg,
  };

  const resize = () => {
    const el = ref.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${el.scrollHeight}px`;
  };

  useLayoutEffect(() => {
    if (!editing) return;
    resize();
    const el = ref.current;
    if (!el) return;
    const caret = focusRequest ? focusRequest.caret : el.value.length;
    el.focus();
    el.setSelectionRange(caret, caret);
    if (focusRequest) onFocusHandled();
  }, [editing, focusRequest, onFocusHandled]);

  const onKeyDown = (e) => {
    const el = e.currentTarget;
    const mod = e.metaKey || e.ctrlKey;
    const atStart = el.selectionStart === 0 && el.selectionEnd === 0;

    if (e.key === 'Escape') {
      e.preventDefault();
      onExit();
      return;
    }
    if (mod && e.key.toLowerCase() === 'b') {
      e.preventDefault();
      applyInline(el, '**', onChange);
      return;
    }
    if (mod && e.key.toLowerCase() === 'i') {
      e.preventDefault();
      applyInline(el, '*', onChange);
      return;
    }
    if (e.key === 'Backspace' && atStart) {
      e.preventDefault();
      onMergeBackward();
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey && !mod) {
      const continued = el.selectionStart === el.selectionEnd ? continueList(el.value, el.selectionStart) : null;
      if (continued) {
        e.preventDefault();
        onChange(continued.value);
        requestAnimationFrame(() => {
          ref.current?.setSelectionRange(continued.caret, continued.caret);
          resize();
        });
        return;
      }
      if (block.type !== 'code' && block.type !== 'table') {
        e.preventDefault();
        onSplit(el.selectionStart);
      }
    }
  };

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
      {editing ? (
        <textarea
          ref={ref}
          className="docblock__editor"
          value={block.md ?? ''}
          onChange={(e) => {
            onChange(e.target.value);
            resize();
          }}
          onKeyDown={onKeyDown}
          onBlur={onExit}
          spellCheck={false}
          placeholder="내용을 입력하세요. 마크다운을 그대로 쓸 수 있습니다."
        />
      ) : (
        <BlockContent block={block} folder={folder} highlight={highlight} />
      )}

      {selected && block.override && <span className="docblock__badge">meta.json</span>}
    </div>
  );
}

/** Rendered view of one block: chart, image, or markdown. */
function BlockContent({ block, folder, width, highlight }) {
  const chart = parseChartBlock(block.md);
  if (chart) {
    return <ChartView spec={chart} width={width ?? 620} height={Math.round((width ?? 620) * 0.55)} />;
  }
  if (!block.md?.trim()) return <p style={{ color: '#b9b7b5' }}>빈 문단</p>;

  const html = renderMarkdown(block.md, {
    assetResolver: (src) => (isProjectAsset(src) ? assetUrl(folder, src) : src),
  });
  return <div dangerouslySetInnerHTML={{ __html: highlight ? markHtml(html, highlight) : html }} />;
}

/** Wrap search hits in the rendered HTML, skipping tag interiors. */
function markHtml(html, query) {
  if (!query) return html;
  const parts = html.split(/(<[^>]*>)/);
  const needle = query.toLowerCase();
  return parts
    .map((part) => {
      if (part.startsWith('<')) return part;
      let out = '';
      let rest = part;
      for (;;) {
        const at = rest.toLowerCase().indexOf(needle);
        if (at === -1) {
          out += rest;
          break;
        }
        out += `${rest.slice(0, at)}<mark class="findhit">${rest.slice(at, at + query.length)}</mark>`;
        rest = rest.slice(at + query.length);
      }
      return out;
    })
    .join('');
}

function applyInline(el, marker, onChange) {
  const result = toggleWrap(el.value, el.selectionStart, el.selectionEnd, marker);
  onChange(result.value);
  requestAnimationFrame(() => el.setSelectionRange(result.start, result.end));
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

function makeTable(rows, cols) {
  const header = `| ${Array.from({ length: cols }, (_, i) => `열 ${i + 1}`).join(' | ')} |`;
  const divider = `|${Array.from({ length: cols }, () => '---').join('|')}|`;
  const body = Array.from(
    { length: Math.max(1, rows) },
    (_, r) => `| ${Array.from({ length: cols }, (_, c) => `${r + 1}-${c + 1}`).join(' | ')} |`
  );
  return [header, divider, ...body].join('\n');
}
