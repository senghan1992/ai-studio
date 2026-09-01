import { positionPhrase, readingOrder } from './geometry.js';
import { plainText, countWords, headingLevel } from './mdblocks.js';
import { usedRange, summarizeColumns, summaryScope } from './grid.js';
import { parseChartBlock, resolveChartSpec, describeChart, chartToMarkdownTable } from './chart.js';
import { indexToCol, toRef, displayValue, recalcSheet } from '@ai-studio/formula';

const KIND_LABEL = {
  text: '텍스트',
  image: '이미지',
  shape: '도형',
  table: '표',
  chart: '차트',
};

/**
 * Build `AI.md` — a single markdown file that flattens a whole project for LLM
 * consumption.
 *
 * The design bet: a model reasons far better about "상단 중앙, 전체 폭" than about
 * `x=96 y=220 w=1088`, and better about a sheet's column summary than about 500
 * rows of numbers. So the digest is not a dump of the source files; it is a
 * translation of them into prose a model already understands, with enough
 * structure (slide numbers, cell addresses, heading levels) to stay verifiable
 * against the source.
 */
export function buildDigest(project) {
  switch (project.type) {
    case 'deck':
      return deckDigest(project);
    case 'doc':
      return docDigest(project);
    case 'grid':
      return gridDigest(project);
    default:
      return `# ${project.manifest?.title ?? 'Untitled'}\n`;
  }
}

function header(project, subtitle) {
  const m = project.manifest ?? {};
  const modified = m.modified ? new Date(m.modified).toISOString().slice(0, 10) : '—';
  return [
    `# ${m.title ?? '제목 없음'}`,
    '',
    `> ${subtitle} · 최종 수정 ${modified}`,
    '',
    '<!-- 이 파일은 저장할 때마다 자동 생성됩니다. 직접 편집하면 다음 저장 시 덮어써집니다. -->',
    '',
  ];
}

/* -------------------------------------------------------------------- deck */

function deckDigest(project) {
  const slides = project.slides ?? [];
  const lines = header(project, `AI Studio 프레젠테이션 · 슬라이드 ${slides.length}장`);

  lines.push('## 목차', '');
  slides.forEach((slide, i) => {
    lines.push(`${i + 1}. ${slide.title || '(제목 없음)'} — \`${slide.layoutName}\``);
  });
  lines.push('');
  lines.push('---', '');

  slides.forEach((slide, i) => {
    lines.push(`## 슬라이드 ${i + 1} — ${slide.title || '(제목 없음)'}`);
    lines.push('');
    lines.push(`- 레이아웃: \`${slide.layoutName}\``);
    lines.push(`- 요소 ${slide.blocks.length}개`);
    if (slide.notes?.trim()) {
      lines.push(`- 발표자 노트: ${collapse(slide.notes)}`);
    }
    lines.push('');

    const ordered = readingOrder(slide.blocks);
    if (!ordered.length) {
      lines.push('_빈 슬라이드_', '');
      return;
    }

    ordered.forEach((block, bi) => {
      const where = positionPhrase(block, slide.canvas);
      const kind = KIND_LABEL[block.kind] ?? block.kind;
      lines.push(`### ${bi + 1}) ${kind} — ${where}`);
      lines.push('');
      const content = String(block.md ?? '').trim();

      // A chart is unreadable as a fenced JSON blob; give the model the shape and
      // the numbers instead.
      const chart = block.kind === 'chart' ? parseChartBlock(content) : null;
      if (chart) {
        lines.push(describeChart(chart));
        lines.push('');
        lines.push(chartToMarkdownTable(chart));
      } else {
        lines.push(content || '_(빈 요소)_');
      }
      lines.push('');
    });

    lines.push('---', '');
  });

  const allText = slides
    .flatMap((s) => s.blocks.map((b) => plainText(b.md)))
    .filter(Boolean)
    .join(' ');
  lines.push('## 전체 통계', '');
  lines.push(`- 슬라이드: ${slides.length}장`);
  lines.push(`- 요소: ${slides.reduce((a, s) => a + s.blocks.length, 0)}개`);
  lines.push(`- 본문 단어: 약 ${countWords(allText)}개`);
  lines.push(`- 발표자 노트가 있는 슬라이드: ${slides.filter((s) => s.notes?.trim()).length}장`);

  return `${lines.join('\n')}\n`;
}

/* --------------------------------------------------------------------- doc */

function docDigest(project) {
  const sections = project.sections ?? [];
  const lines = header(project, `AI Studio 문서 · 섹션 ${sections.length}개`);

  const outline = [];
  sections.forEach((section, si) => {
    section.blocks.forEach((b) => {
      const level = headingLevel(b.md);
      if (level) outline.push({ level, text: plainText(b.md), section: si + 1 });
    });
  });

  if (outline.length) {
    lines.push('## 목차', '');
    for (const item of outline) {
      lines.push(`${'  '.repeat(Math.max(0, item.level - 1))}- ${item.text}`);
    }
    lines.push('');
    lines.push('---', '');
  }

  sections.forEach((section, si) => {
    lines.push(`## 섹션 ${si + 1} — ${section.name}`);
    lines.push('');
    for (const block of section.blocks) {
      const md = String(block.md ?? '').trim();
      if (!md) continue;

      const chart = parseChartBlock(md);
      if (chart) {
        lines.push(describeChart(chart));
        lines.push('');
        lines.push(chartToMarkdownTable(chart));
      } else {
        lines.push(md);
      }
      if (block.override) {
        lines.push(`<!-- 서식: ${describeOverride(block.override)} -->`);
      }
      lines.push('');
    }
    lines.push('---', '');
  });

  const text = sections.flatMap((s) => s.blocks.map((b) => plainText(b.md))).join('\n');
  lines.push('## 전체 통계', '');
  lines.push(`- 섹션: ${sections.length}개`);
  lines.push(`- 단어: 약 ${countWords(text)}개`);
  lines.push(`- 글자(공백 제외): ${text.replace(/\s/g, '').length}자`);
  lines.push(`- 제목: ${outline.length}개`);

  return `${lines.join('\n')}\n`;
}

function describeOverride(o) {
  const parts = [];
  if (o.align) parts.push(`정렬 ${o.align}`);
  if (o.indent) parts.push(`들여쓰기 ${o.indent}px`);
  if (o.style?.italic) parts.push('기울임');
  if (o.style?.bold) parts.push('굵게');
  if (o.style?.color) parts.push(`색 ${o.style.color}`);
  if (o.style?.fontSize) parts.push(`크기 ${o.style.fontSize}px`);
  return parts.join(', ') || '기본';
}

/* -------------------------------------------------------------------- grid */

const GRID_DIGEST_ROWS = 60;

function gridDigest(project) {
  // Recalculate here rather than trusting the caller's cached `v` values: the
  // digest is generated on save, and a caller may hand us cells whose formulas
  // have never been evaluated (an API client, or a freshly parsed file).
  const sheets = (project.sheets ?? []).map((sheet) => ({
    ...sheet,
    cells: recalcSheet({ cells: sheet.cells, names: sheet.names }).cells,
  }));
  const lines = header(project, `AI Studio 스프레드시트 · 시트 ${sheets.length}개`);

  lines.push('## 시트 목록', '');
  sheets.forEach((sheet, i) => {
    const range = usedRange(sheet.cells);
    const size = range ? `${range.maxRow + 1}행 × ${range.maxCol + 1}열` : '빈 시트';
    const formulas = Object.values(sheet.cells).filter((c) => c.f).length;
    lines.push(`${i + 1}. **${sheet.name}** — ${size}, 수식 ${formulas}개`);
  });
  lines.push('');
  lines.push('---', '');

  sheets.forEach((sheet, i) => {
    const range = usedRange(sheet.cells);
    lines.push(`## 시트 ${i + 1} — ${sheet.name}`);
    lines.push('');

    if (!range) {
      lines.push('_빈 시트_', '', '---', '');
      return;
    }

    lines.push(`데이터 범위: \`A1:${toRef(range.maxCol, range.maxRow)}\``);
    lines.push('');

    const summary = summarizeColumns(sheet);
    if (summary.length) {
      const scope = summaryScope(sheet);
      lines.push('### 열 구성', '');
      if (scope) {
        const excluded = scope.excluded.length
          ? `, 집계 행 ${scope.excluded.map((r) => `${r}행`).join('/')}은 제외`
          : '';
        lines.push(`아래 통계는 ${scope.firstRow}–${scope.lastRow}행 데이터 기준입니다${excluded}.`);
        lines.push('');
      }
      for (const col of summary) {
        const stats =
          col.sum !== null
            ? ` (합계 ${col.sum}, 평균 ${col.avg}, 범위 ${col.min}~${col.max})`
            : '';
        lines.push(`- \`${col.col}\` **${col.header}** — ${col.type}, 값 ${col.count}개${stats}`);
      }
      lines.push('');
    }

    // Records read better than a grid when there is a header row.
    const headerRow = [];
    for (let c = 0; c <= range.maxCol; c++) {
      headerRow.push(displayValue(sheet.cells[toRef(c, 0)]) || indexToCol(c));
    }
    const hasHeader = headerRow.some((h, idx) => h !== indexToCol(idx));

    lines.push('### 데이터', '');
    const limit = Math.min(range.maxRow, GRID_DIGEST_ROWS);
    if (hasHeader) {
      for (let r = 1; r <= limit; r++) {
        const parts = [];
        for (let c = 0; c <= range.maxCol; c++) {
          const cell = sheet.cells[toRef(c, r)];
          const shown = displayValue(cell);
          if (shown === '') continue;
          parts.push(`${headerRow[c]}=${shown}`);
        }
        if (parts.length) lines.push(`- ${r + 1}행: ${parts.join(', ')}`);
      }
    } else {
      for (let r = 0; r <= limit; r++) {
        const parts = [];
        for (let c = 0; c <= range.maxCol; c++) {
          const shown = displayValue(sheet.cells[toRef(c, r)]);
          if (shown !== '') parts.push(`${toRef(c, r)}=${shown}`);
        }
        if (parts.length) lines.push(`- ${parts.join(', ')}`);
      }
    }
    if (range.maxRow > GRID_DIGEST_ROWS) {
      lines.push(`- _… ${range.maxRow - GRID_DIGEST_ROWS}개 행 생략_`);
    }
    lines.push('');

    const formulas = Object.entries(sheet.cells).filter(([, c]) => c.f);
    if (formulas.length) {
      lines.push('### 수식과 계산 결과', '');
      for (const [ref, cell] of formulas) {
        lines.push(`- \`${ref}\`: \`${cell.f}\` → **${displayValue(cell) || '(빈 값)'}**`);
      }
      lines.push('');
    }

    if (sheet.charts?.length) {
      lines.push('### 차트', '');
      for (const chart of sheet.charts) {
        const resolved = resolveChartSpec(chart.spec, sheet);
        lines.push(`- ${describeChart(resolved)}`);
        lines.push('');
        lines.push(chartToMarkdownTable(resolved));
        lines.push('');
      }
    }

    if (Object.keys(sheet.names ?? {}).length) {
      lines.push('### 이름 있는 범위', '');
      for (const [name, target] of Object.entries(sheet.names)) lines.push(`- \`${name}\` → \`${target}\``);
      lines.push('');
    }

    lines.push('---', '');
  });

  return `${lines.join('\n')}\n`;
}

function collapse(text) {
  return String(text).replace(/\s+/g, ' ').trim();
}

