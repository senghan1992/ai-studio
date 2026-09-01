import React, { useCallback, useEffect, useMemo, useState } from 'react';
import {
  makeSlide, newBlockId, SLIDE_LAYOUTS, positionPhrase, autoLayout,
  serializeChartBlock, parseChartBlock,
} from '../core/index.js';

import Shell from '../components/Shell.jsx';
import FileInspector from '../components/FileInspector.jsx';
import FileMenu from '../components/FileMenu.jsx';
import ChartDialog from '../components/ChartDialog.jsx';
import ImageDialog from '../components/ImageDialog.jsx';
import {
  Ribbon, Group, Btn, Select, NumInput, ColorRow,
  ContextMenu, useContextMenu, ZoomSlider,
} from '../components/ui.jsx';
import SlideCanvas from './SlideCanvas.jsx';
import SlideSorter from './SlideSorter.jsx';
import Slideshow from './Slideshow.jsx';
import { toggleWrap, toggleLinePrefix } from '../lib/markdown.js';
import { alignBlocks, distributeBlocks } from './blockOps.js';

const TABS = ['파일', '홈', '삽입', '디자인', '슬라이드 쇼', 'AI'];
const TEXT_COLORS = ['#111827', '#4f46e5', '#c43e1c', '#107c41', '#b45309', '#6b7280'];
const FILL_COLORS = ['#e5e7eb', '#dbeafe', '#dcfce7', '#fee2e2', '#fef3c7', '#ede9fe'];
const LAYOUT_LABELS = {
  title: '표지',
  'title-content': '제목 + 내용',
  'two-column': '2단',
  section: '섹션 구분',
  blank: '빈 슬라이드',
};

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

  /* --------------------------------------------------------- slide commands */

  const addSlide = (layoutName = 'title-content') => {
    const next = makeSlide(layoutName, { index: slides.length + 1 });
    setItems((list) => [...list.slice(0, slideIndex + 1), next, ...list.slice(slideIndex + 1)]);
    setCurrent(slideIndex + 1);
  };

  const duplicateSlide = () => {
    if (!slide) return;
    const copy = {
      ...slide,
      id: undefined,
      title: `${slide.title} 사본`,
      blocks: slide.blocks.map((b) => ({ ...b, id: newBlockId() })),
    };
    setItems((list) => [...list.slice(0, slideIndex + 1), copy, ...list.slice(slideIndex + 1)]);
    setCurrent(slideIndex + 1);
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
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
      const mod = e.metaKey || e.ctrlKey;

      if (e.key === 'F5') {
        e.preventDefault();
        setPresenting(true);
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
  }, [presenting, selectedId, clipboard, duplicateBlock, copyBlock, pasteBlock, slides.length]);

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
    '-',
    ...(block.kind === 'chart' || block.kind === 'image'
      ? [{ label: block.kind === 'chart' ? '차트 편집' : '이미지 편집', onClick: () => openBlock(block) }]
      : [{ label: '텍스트 편집', shortcut: 'Enter', onClick: () => setEditingId(block.id) }]),
    '-',
    { label: '맨 앞으로', onClick: () => patchBlock(block.id, { z: maxZ(slide) + 1 }) },
    { label: '맨 뒤로', onClick: () => patchBlock(block.id, { z: minZ(slide) - 1 }) },
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
    { label: '복제', onClick: () => { setCurrent(at); duplicateSlide(); } },
    '-',
    { label: '위로 이동', disabled: at === 0, onClick: () => moveSlide(at, at - 1) },
    { label: '아래로 이동', disabled: at === slides.length - 1, onClick: () => moveSlide(at, at + 1) },
    '-',
    { label: '여기서 슬라이드 쇼', onClick: () => { setCurrent(at); setPresenting(true); } },
    '-',
    { label: '삭제', danger: true, disabled: slides.length <= 1, onClick: () => deleteSlide(at) },
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
            </div>
          </Group>

          <Group label="슬라이드">
            <Btn icon="🞤" label="새 슬라이드" onClick={() => addSlide()} />
            <Btn icon="⧉" label="복제" onClick={duplicateSlide} />
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
              <ColorRow colors={TEXT_COLORS} value={selected?.style?.color} onChange={(c) => selected && patchStyle(selected.id, { color: c })} title="글자 색" />
              <ColorRow colors={FILL_COLORS} value={selected?.style?.fill} onChange={(c) => selected && patchStyle(selected.id, { fill: c })} title="채우기 색" />
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
                  <Btn key={value} small icon={icon} label="" title={`문단 ${title} 정렬`} disabled={!selected} pressed={selected?.style?.align === value} onClick={() => patchStyle(selected.id, { align: value })} />
                ))}
                {[
                  ['top', '⤒', '위'],
                  ['middle', '⇳', '중간'],
                  ['bottom', '⤓', '아래'],
                ].map(([value, icon, title]) => (
                  <Btn key={value} small icon={icon} label="" title={`세로 ${title}`} disabled={!selected} pressed={selected?.style?.valign === value} onClick={() => patchStyle(selected.id, { valign: value })} />
                ))}
              </div>
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
            </div>
          </Group>

          <Group label="배치">
            <div className="rcol">
              <div className="rrow">
                <Btn small icon="⬆" label="맨 앞" disabled={!selected} onClick={() => patchBlock(selected.id, { z: maxZ(slide) + 1 })} />
                <Btn small icon="⬇" label="맨 뒤" disabled={!selected} onClick={() => patchBlock(selected.id, { z: minZ(slide) - 1 })} />
              </div>
              <div className="rrow">
                <Btn small icon="⋮" label="세로 분배" disabled={slide.blocks.length < 3} onClick={() => doDistribute('vertical')} />
                <Btn small icon="⋯" label="가로 분배" disabled={slide.blocks.length < 3} onClick={() => doDistribute('horizontal')} />
              </div>
            </div>
          </Group>
        </>
      )}

      {tab === '삽입' && (
        <>
          <Group label="텍스트">
            <Btn icon="T" label="텍스트 상자" onClick={() => addBlock({ md: '새 텍스트' })} />
            <Btn icon="H" label="제목" onClick={() => addBlock({ md: '# 제목', style: { fontSize: 36, weight: 700, align: 'left' }, h: 80 })} />
            <Btn icon="≔" label="목록" onClick={() => addBlock({ md: '- 항목 1\n- 항목 2', h: 160 })} />
          </Group>
          <Group label="그림 · 차트">
            <Btn icon="🖼" label="이미지" onClick={() => setImageDialog({ src: '', alt: '' })} />
            <Btn icon="📊" label="차트" onClick={() => setChartDialog({ spec: null })} />
          </Group>
          <Group label="개체">
            <Btn
              icon="▦"
              label="표"
              onClick={() => addBlock({ kind: 'table', md: '| 항목 | 값 |\n|---|---|\n| 가 | 1 |\n| 나 | 2 |', w: 520, h: 200 })}
            />
            <Btn
              icon="◼"
              label="도형"
              onClick={() =>
                addBlock({
                  kind: 'shape',
                  md: '',
                  w: 240,
                  h: 140,
                  style: { fill: '#dbeafe', radius: 8, align: 'center', valign: 'middle', fontSize: 18 },
                })
              }
            />
            <Btn icon="▤" label="코드" onClick={() => addBlock({ md: '```js\nconst x = 1;\n```', w: 520, h: 160 })} />
          </Group>
        </>
      )}

      {tab === '디자인' && (
        <>
          <Group label="슬라이드 배경">
            <ColorRow
              colors={['#ffffff', '#f8fafc', '#111827', '#1e293b', '#fef9c3', '#eef2ff']}
              value={slide.canvas.bg}
              onChange={(bg) => patchSlide((s) => ({ ...s, canvas: { ...s.canvas, bg } }))}
              title="배경색"
            />
          </Group>
          <Group label="모든 슬라이드 배경">
            <ColorRow
              colors={['#ffffff', '#f8fafc', '#111827', '#1e293b']}
              value={undefined}
              onChange={(bg) => setItems((list) => list.map((s) => ({ ...s, canvas: { ...s.canvas, bg } })))}
              title="모든 슬라이드 배경색"
            />
          </Group>
          <Group label="테마 강조색">
            <ColorRow
              colors={['#4f46e5', '#c43e1c', '#107c41', '#185abd', '#b45309', '#0f766e']}
              value={project.manifest?.theme?.accent}
              onChange={(accent) =>
                ctl.commit((p) => ({ ...p, manifest: { ...p.manifest, theme: { ...p.manifest.theme, accent } } }))
              }
              title="강조색"
            />
          </Group>
          <Group label="캔버스 크기">
            <Select
              value={`${slide.canvas.w}x${slide.canvas.h}`}
              onChange={(v) => {
                const [w, h] = v.split('x').map(Number);
                setItems((list) => list.map((s) => ({ ...s, canvas: { ...s.canvas, w, h } })));
              }}
              options={[
                { value: '1280x720', label: '16:9 (1280×720)' },
                { value: '1600x900', label: '16:9 (1600×900)' },
                { value: '1024x768', label: '4:3 (1024×768)' },
              ]}
              title="캔버스 크기"
              width={150}
            />
          </Group>
        </>
      )}

      {tab === '슬라이드 쇼' && (
        <>
          <Group label="시작">
            <Btn icon="▶" label="처음부터" title="F5" onClick={() => { setCurrent(0); setPresenting(true); }} />
            <Btn icon="▷" label="현재 슬라이드부터" onClick={() => setPresenting(true)} />
          </Group>
          <Group label="발표자 노트">
            <div style={{ fontSize: 11.5, color: 'var(--ink-2)', maxWidth: 380, lineHeight: 1.6, padding: '2px 6px' }}>
              쇼 중에 <kbd>S</kbd>를 누르면 발표자 노트가 열립니다. 이동은 <kbd>→</kbd>/<kbd>Space</kbd>,
              종료는 <kbd>Esc</kbd>.
              <br />
              노트가 있는 슬라이드: {slides.filter((s) => s.notes?.trim()).length} / {slides.length}
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
          }}
          onEdit={setEditingId}
          onChangeBlock={changeBlockBox}
          onChangeBlockMd={changeBlockMd}
          onAddBlock={addBlock}
          onDeleteBlock={deleteBlock}
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
