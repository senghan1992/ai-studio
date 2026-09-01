/**
 * Create three realistic sample projects in the workspace.
 *
 * The point is to have something worth reading in `AI.md` — a deck with speaker
 * notes, a doc with real structure, a sheet with a formula chain — so the storage
 * format can be judged on real content rather than placeholder text.
 *
 *   npm run seed
 */
import path from 'node:path';
import { promises as fs } from 'node:fs';
import { fileURLToPath } from 'node:url';

import {
  createProject, loadProject, saveProject,
  makeSlide, makeSection, makeSheet, newBlockId,
  serializeChartBlock,
} from '../packages/format/src/index.js';
import { toRef, indexToCol } from '../packages/formula/src/index.js';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const WORKSPACE = path.resolve(process.env.AI_STUDIO_WORKSPACE ?? path.join(ROOT, 'workspace'));

await fs.mkdir(WORKSPACE, { recursive: true });

const text = (md, box, style = {}) => ({
  id: newBlockId(),
  kind: 'text',
  md,
  ...box,
  style: { fontSize: 20, weight: 400, align: 'left', color: '#1f2937', lineHeight: 1.45, ...style },
});

const chart = (spec, box) => ({
  id: newBlockId(),
  kind: 'chart',
  md: serializeChartBlock(spec),
  ...box,
  style: {},
});

/* ------------------------------------------------------------------- deck */

{
  const project = await createProject(WORKSPACE, {
    type: 'deck',
    title: '2026 3분기 사업 리뷰',
    sample: false,
  });

  const canvas = { w: 1280, h: 720, bg: '#ffffff' };

  project.slides = [
    {
      ...makeSlide('title'),
      title: '2026 3분기 사업 리뷰',
      layoutName: 'title',
      notes: '인사 후 바로 핵심 숫자로 들어간다. 2분 이내.',
      canvas,
      blocks: [
        text('# 2026 3분기 사업 리뷰', { x: 96, y: 250, w: 1088, h: 110, z: 1 },
          { fontSize: 52, weight: 700, align: 'center' }),
        text('매출 성장 · 신규 시장 진입 · 4분기 계획', { x: 96, y: 376, w: 1088, h: 50, z: 2 },
          { fontSize: 22, align: 'center', color: '#6b7280' }),
        text('전략기획팀 · 2026-10-14', { x: 96, y: 620, w: 1088, h: 34, z: 3 },
          { fontSize: 14, align: 'center', color: '#9ca3af' }),
      ],
    },
    {
      ...makeSlide('title-content'),
      title: '핵심 성과',
      layoutName: 'title-content',
      notes: '전년 동기 대비라는 점을 반드시 짚는다. 이탈률은 질문이 나오면 4번 슬라이드에서 다룬다고 안내.',
      canvas,
      blocks: [
        text('# 핵심 성과', { x: 96, y: 64, w: 1088, h: 70, z: 1 }, { fontSize: 38, weight: 700 }),
        text(
          '- **매출 142억** — 전년 동기 대비 +24%\n- **신규 고객 1,280개사** — 목표의 106%\n- **월 이탈률 1.8%** — 전분기 2.4%에서 개선\n- **영업이익률 11.2%** — 4개 분기 연속 상승',
          { x: 96, y: 176, w: 620, h: 300, z: 2 },
          { fontSize: 21, lineHeight: 1.85 }
        ),
        text(
          '| 지표 | 2분기 | 3분기 |\n|---|---|---|\n| 매출 | 114억 | 142억 |\n| 신규 고객 | 980 | 1,280 |\n| 이탈률 | 2.4% | 1.8% |',
          { x: 756, y: 176, w: 428, h: 220, z: 3 },
          { fontSize: 16 }
        ),
        text('_출처: 재무팀 확정 실적 (2026-10-10)_', { x: 756, y: 420, w: 428, h: 34, z: 4 },
          { fontSize: 13, color: '#9ca3af' }),
      ],
    },
    {
      ...makeSlide('title-content'),
      title: '매출 추이',
      layoutName: 'title-content',
      notes: '3분기 급증은 엔터프라이즈 계약 두 건이 겹친 결과다. 4분기는 보수적으로 본다.',
      canvas,
      blocks: [
        text('# 매출 추이', { x: 96, y: 64, w: 1088, h: 70, z: 1 }, { fontSize: 38, weight: 700 }),
        chart(
          {
            type: 'column',
            title: '분기별 매출 (억원)',
            labels: ['1분기', '2분기', '3분기', '4분기(계획)'],
            series: [
              { name: '2025', values: [86, 98, 105, 118] },
              { name: '2026', values: [112, 114, 142, 160] },
            ],
            options: { valueLabels: 'max' },
          },
          { x: 96, y: 168, w: 620, h: 420, z: 2 }
        ),
        chart(
          {
            type: 'donut',
            title: '3분기 매출 구성',
            labels: ['직접 영업', '파트너', '온라인'],
            series: [{ name: '매출', values: [78, 41, 23] }],
          },
          { x: 748, y: 168, w: 436, h: 420, z: 3 }
        ),
      ],
    },
    {
      ...makeSlide('two-column'),
      title: '신규 시장 진입 현황',
      layoutName: 'two-column',
      notes: '동남아는 예상보다 빠르다. 일본은 파트너 계약이 지연된 상태라 4분기 리스크로 언급.',
      canvas,
      blocks: [
        text('# 신규 시장 진입 현황', { x: 96, y: 64, w: 1088, h: 70, z: 1 }, { fontSize: 38, weight: 700 }),
        text(
          '## 동남아시아\n\n- 싱가포르·베트남 법인 설립 완료\n- 첫 매출 8.4억 발생\n- 현지 채용 14명',
          { x: 96, y: 180, w: 512, h: 380, z: 2 },
          { fontSize: 19, lineHeight: 1.7 }
        ),
        text(
          '## 일본\n\n- 파트너 계약 **지연** (11월 예상)\n- 사전 문의 62건 확보\n- 4분기 리스크 항목',
          { x: 672, y: 180, w: 512, h: 380, z: 3 },
          { fontSize: 19, lineHeight: 1.7 }
        ),
      ],
    },
    {
      ...makeSlide('section'),
      title: '4분기 계획',
      layoutName: 'section',
      notes: '여기서 잠깐 멈추고 질문을 받는다.',
      canvas: { ...canvas, bg: '#111827' },
      blocks: [
        text('## 4분기 계획', { x: 96, y: 300, w: 1088, h: 100, z: 1 },
          { fontSize: 44, weight: 600, align: 'center', color: '#ffffff' }),
        text('세 가지 우선순위', { x: 96, y: 412, w: 1088, h: 44, z: 2 },
          { fontSize: 20, align: 'center', color: '#9ca3af' }),
      ],
    },
    {
      ...makeSlide('title-content'),
      title: '4분기 우선순위',
      layoutName: 'title-content',
      notes: '각 항목의 담당 조직을 확실히 밝힌다. 예산은 별도 시트 참조.',
      canvas,
      blocks: [
        text('# 4분기 우선순위', { x: 96, y: 64, w: 1088, h: 70, z: 1 }, { fontSize: 38, weight: 700 }),
        text(
          '1. **일본 파트너 계약 마무리** — 사업개발팀, 11월 15일\n2. **엔터프라이즈 요금제 출시** — 프로덕트팀, 12월 1일\n3. **이탈률 1.5% 달성** — 고객성공팀, 분기 내',
          { x: 96, y: 190, w: 1088, h: 240, z: 2 },
          { fontSize: 22, lineHeight: 2 }
        ),
        text(
          '> 예산 상세는 `2026-4분기-예산.aigrid` 프로젝트를 참조하세요.',
          { x: 96, y: 470, w: 1088, h: 60, z: 3 },
          { fontSize: 17, color: '#4f46e5' }
        ),
      ],
    },
  ];

  await saveProject(project);
  console.log(`✓ Deck   ${path.basename(project.dir)} (슬라이드 ${project.slides.length}장)`);
}

/* -------------------------------------------------------------------- doc */

{
  const project = await createProject(WORKSPACE, {
    type: 'doc',
    title: 'AI Studio 저장 포맷 제안서',
    sample: false,
  });

  const block = (md, override = null) => ({ id: newBlockId(), md, type: 'paragraph', override });

  project.sections = [
    {
      ...makeSection({ name: '요약' }),
      name: '요약',
      blocks: [
        block('# AI Studio 저장 포맷 제안서'),
        block(
          '기존 오피스 포맷(`.pptx`, `.docx`, `.xlsx`)은 zip 컨테이너 안에 수십 개의 XML 파트를 담는다. 사람이 쓰기에는 문제가 없지만, LLM이 읽기에는 세 가지 문제가 있다.'
        ),
        block(
          '1. 텍스트가 런(run) 단위로 조각나 문장이 끊긴다\n2. 위치 정보가 EMU 단위 좌표로만 존재해 의미를 알 수 없다\n3. 파일 전체가 하나의 바이너리라 청킹 단위가 없다'
        ),
        block(
          '> 이 제안서는 내용을 Markdown에, 배치와 계산을 JSON에 나누어 담는 폴더 기반 포맷을 정의한다.',
          { align: 'center', style: { italic: true, color: '#4f46e5' } }
        ),
      ],
    },
    {
      ...makeSection({ name: '설계 원칙' }),
      name: '설계 원칙',
      blocks: [
        block('# 설계 원칙'),
        block('## 1. 프로젝트는 폴더다'),
        block(
          '단일 zip 대신 일반 디렉터리를 쓴다. `git diff`가 동작하고, `grep`이 동작하며, RAG 인덱서가 슬라이드나 섹션 단위로 청킹할 수 있다.'
        ),
        block('## 2. 내용은 Markdown, 배치는 JSON'),
        block(
          '사람이 읽는 텍스트는 `.md`에, 좌표·스타일·수식은 `.json`에 넣고 둘을 **블록 ID**로 연결한다. 어느 한쪽만 읽어도 의미가 통한다는 점이 핵심이다.'
        ),
        block('## 3. Markdown은 손으로 고칠 수 있다'),
        block(
          '`.md`만 편집해도 앱이 열린다. 배치 정보가 없는 블록은 자동 레이아웃으로 채워지므로, AI 에이전트가 마크다운만 써서 슬라이드를 추가하는 것도 가능하다.'
        ),
        block('## 4. AI.md는 자동 생성 다이제스트'),
        block(
          '저장할 때마다 프로젝트 전체를 한 파일로 평탄화한다. 이때 좌표는 픽셀이 아니라 `상단 중앙`, `좌측 하단` 같은 자연어로 번역된다 — 모델이 훨씬 잘 이해하는 표현이다.'
        ),
      ],
    },
    {
      ...makeSection({ name: '비교' }),
      name: '비교',
      blocks: [
        block('# 기존 포맷과의 비교'),
        block(
          '| 항목 | OOXML | AI Studio |\n|---|---|---|\n| 컨테이너 | zip + XML 파트 | 일반 폴더 |\n| 텍스트 추출 | XML 파싱 필요 | md 그대로 |\n| 위치 정보 | EMU 좌표 | JSON + 자연어 |\n| 청킹 단위 | 파일 전체 | 슬라이드/섹션/시트 |\n| 버전 관리 | 바이너리 diff | 텍스트 diff |'
        ),
        block('## 절충점'),
        block(
          '폴더 기반이므로 단일 파일 첨부가 어렵다. 배포가 필요하면 폴더를 zip으로 묶되, 편집 중에는 항상 펼쳐진 상태를 유지한다.'
        ),
        block(
          '들여쓴 본문 예시입니다. 이 문단만 서식이 지정되어 있으므로 `.meta.json`에 항목이 하나 생깁니다.',
          { indent: 48, style: { fontSize: 14, color: '#6b7280' } }
        ),
        block('## 청킹 효율'),
        block(
          '같은 문서를 두 포맷으로 저장하고 RAG 인덱서에 넣었을 때, 청크당 잡히는 의미 단위를 비교한 결과다.'
        ),
        block(
          serializeChartBlock({
            type: 'bar',
            title: '청크 1개에 담기는 의미 단위 수',
            labels: ['OOXML 텍스트 추출', 'AI Studio .md', 'AI Studio AI.md'],
            series: [{ name: '의미 단위', values: [1.4, 3.8, 6.2] }],
            options: { valueLabels: 'all' },
          })
        ),
      ],
    },
  ];

  await saveProject(project);
  console.log(`✓ Doc    ${path.basename(project.dir)} (섹션 ${project.sections.length}개)`);
}

/* ------------------------------------------------------------------- grid */

{
  const project = await createProject(WORKSPACE, {
    type: 'grid',
    title: '2026 4분기 예산',
    sample: false,
  });

  const cells = {};
  const put = (col, row, cell) => {
    cells[toRef(col, row)] = cell;
  };

  const headers = ['항목', '10월', '11월', '12월', '분기 합계', '비중'];
  headers.forEach((h, c) =>
    put(c, 0, { v: h, t: 's', style: { bold: true, bg: '#e8f5ee', align: 'center' } })
  );

  const rows = [
    ['인건비', 480000000, 495000000, 495000000],
    ['마케팅', 210000000, 260000000, 320000000],
    ['인프라', 88000000, 92000000, 96000000],
    ['일본 진출', 140000000, 180000000, 60000000],
    ['기타 운영', 46000000, 48000000, 52000000],
  ];

  rows.forEach(([label, oct, nov, dec], i) => {
    const r = i + 1;
    const excelRow = r + 1;
    put(0, r, { v: label, t: 's' });
    put(1, r, { v: oct, t: 'n', fmt: '₩#,##0' });
    put(2, r, { v: nov, t: 'n', fmt: '₩#,##0' });
    put(3, r, { v: dec, t: 'n', fmt: '₩#,##0' });
    put(4, r, { f: `=SUM(B${excelRow}:D${excelRow})`, t: 'n', fmt: '₩#,##0' });
    put(5, r, { f: `=E${excelRow}/$E$${rows.length + 2}`, t: 'n', fmt: '0.0%' });
  });

  const totalRow = rows.length + 1;
  const totalExcelRow = totalRow + 1;
  put(0, totalRow, { v: '합계', t: 's', style: { bold: true, bg: '#f8fafc' } });
  for (let c = 1; c <= 4; c++) {
    const col = indexToCol(c);
    put(c, totalRow, {
      f: `=SUM(${col}2:${col}${totalRow})`,
      t: 'n',
      fmt: '₩#,##0',
      style: { bold: true, bg: '#f8fafc' },
    });
  }
  put(5, totalRow, {
    f: `=SUM(F2:F${totalRow})`,
    t: 'n',
    fmt: '0.0%',
    style: { bold: true, bg: '#f8fafc' },
  });

  // A small analysis block below the table, the way a real budget sheet has one.
  const analysisRow = totalRow + 2;
  put(0, analysisRow, { v: '분석', t: 's', style: { bold: true } });
  put(0, analysisRow + 1, { v: '최대 지출 항목', t: 's' });
  put(1, analysisRow + 1, { f: `=INDEX(A2:A${totalRow},MATCH(MAX(E2:E${totalRow}),E2:E${totalRow},0))`, t: 's' });
  put(0, analysisRow + 2, { v: '월 평균 지출', t: 's' });
  put(1, analysisRow + 2, { f: `=E${totalExcelRow}/3`, t: 'n', fmt: '₩#,##0' });
  put(0, analysisRow + 3, { v: '마케팅 증가율 (10월→12월)', t: 's' });
  put(1, analysisRow + 3, { f: '=D3/B3-1', t: 'n', fmt: '0.0%' });

  project.sheets = [
    {
      ...makeSheet({ name: '예산' }),
      name: '예산',
      cells,
      frozen: { rows: 1, cols: 1 },
      colWidths: { A: 190, B: 130, C: 130, D: 130, E: 140, F: 80 },
      names: { 예산표: `A1:F${totalExcelRow}`, 월별지출: `B2:D${totalRow}` },
      // A range-backed chart: edit a number and it redraws, and its figures land
      // in the sheet's markdown so a model reads values rather than a picture.
      charts: [
        {
          id: newBlockId(),
          x: 60,
          y: 300,
          w: 620,
          h: 340,
          spec: {
            type: 'column',
            title: '항목별 월 지출',
            range: `A1:D${totalRow}`,
            orientation: 'columns',
            options: { stacked: false, valueLabels: 'max' },
          },
        },
        {
          id: newBlockId(),
          x: 720,
          y: 300,
          w: 420,
          h: 340,
          spec: {
            // Two columns: labels from A, one series from B — a pie needs exactly one.
            type: 'pie',
            title: '10월 지출 비중',
            range: `A1:B${totalRow}`,
            orientation: 'columns',
            options: { legend: true },
          },
        },
      ],
    },
    {
      ...makeSheet({ name: '가정' }),
      name: '가정',
      cells: {
        A1: { v: '가정', t: 's', style: { bold: true, bg: '#e8f5ee' } },
        B1: { v: '값', t: 's', style: { bold: true, bg: '#e8f5ee' } },
        A2: { v: '환율 (JPY/KRW)', t: 's' },
        B2: { v: 9.2, t: 'n', fmt: '#,##0.00' },
        A3: { v: '인건비 인상률', t: 's' },
        B3: { v: 0.031, t: 'n', fmt: '0.0%' },
        A4: { v: '목표 영업이익률', t: 's' },
        B4: { v: 0.112, t: 'n', fmt: '0.0%' },
        A5: { v: '기준일', t: 's' },
        B5: { v: 46296, t: 'd', fmt: 'yyyy-mm-dd' },
      },
      frozen: { rows: 1, cols: 0 },
      colWidths: { A: 200, B: 130 },
    },
  ];

  await saveProject(project);
  console.log(`✓ Grid   ${path.basename(project.dir)} (시트 ${project.sheets.length}개)`);
}

console.log(`\n샘플 프로젝트를 만들었습니다: ${WORKSPACE}`);
