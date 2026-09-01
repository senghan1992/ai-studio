/**
 * Verify the Office exports produce real, well-formed files.
 *
 * A .pptx/.docx/.xlsx is a zip of XML parts, so we can check structure without a
 * copy of Office: the zip must open, the required parts must exist, and the
 * document's own text must appear in the XML. That catches the failure that
 * matters — an export that "succeeds" but contains nothing.
 *
 *   ./scripts/restart-server.sh && node scripts/export-check.mjs
 */
import { promises as fs } from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const run = promisify(execFile);
const BASE = process.env.AI_STUDIO_API ?? 'http://localhost:5177';

let failures = 0;
let checks = 0;
const check = (label, ok, detail) => {
  checks++;
  if (ok) console.log(`  ✓ ${label}`);
  else {
    failures++;
    console.log(`  ✗ ${label}${detail ? `\n      ${String(detail).slice(0, 400)}` : ''}`);
  }
};

async function call(p, options) {
  const res = await fetch(`${BASE}${p}`, {
    headers: options?.body ? { 'Content-Type': 'application/json' } : undefined,
    ...options,
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${options?.method ?? 'GET'} ${p} → ${res.status}: ${text.slice(0, 200)}`);
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

async function download(p) {
  const res = await fetch(`${BASE}${p}`);
  if (!res.ok) throw new Error(`GET ${p} → ${res.status}: ${(await res.text()).slice(0, 200)}`);
  return {
    buffer: Buffer.from(await res.arrayBuffer()),
    contentType: res.headers.get('content-type') ?? '',
    disposition: res.headers.get('content-disposition') ?? '',
  };
}

const tmp = await fs.mkdtemp(path.join(os.tmpdir(), 'ai-studio-export-'));

const { workspace } = await call('/api/health');
/** Read a file straight from the project folder on disk. */
const read = (folder, rel) => fs.readFile(path.join(workspace, folder, rel), 'utf8');

/**
 * List a zip's entries and concatenate its XML text.
 *
 * Everything is extracted to a directory rather than fetched entry-by-entry:
 * `unzip -p` treats the entry name as a glob, and OOXML's `[Content_Types].xml`
 * is nothing but glob metacharacters.
 */
async function readZip(file) {
  const listing = await run('unzip', ['-Z1', file]);
  const entries = listing.stdout.trim().split('\n');

  const dir = `${file}.extracted`;
  await run('unzip', ['-oq', file, '-d', dir]);

  let xml = '';
  for (const entry of entries) {
    if (!/\.(xml|rels)$/.test(entry)) continue;
    xml += await fs.readFile(path.join(dir, entry), 'utf8').catch(() => '');
  }
  return { entries, xml };
}

const hasUnzip = await run('which', ['unzip']).then(() => true).catch(() => false);
if (!hasUnzip) {
  console.error('unzip이 필요합니다 (zip 구조 검사용).');
  process.exit(2);
}

/* ------------------------------------------------------------------- deck */

console.log('\n■ Deck → .pptx');
{
  const created = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'deck', title: '내보내기 덱', sample: false }),
  });
  const folder = created.folder;

  const slide = created.slides[0];
  slide.notes = '이 슬라이드의 발표자 노트';
  slide.blocks = [
    {
      id: 'b1', kind: 'text', md: '# 분기 실적 요약',
      x: 96, y: 80, w: 1088, h: 90, z: 1,
      style: { fontSize: 40, weight: 700, align: 'center', color: '#111827' },
    },
    {
      id: 'b2', kind: 'text', md: '- 매출 **142억** 달성\n- 신규 고객 1,280개사\n- *이탈률 개선*',
      x: 96, y: 220, w: 600, h: 260, z: 2, style: { fontSize: 20 },
    },
    {
      id: 'b3', kind: 'table', md: '| 지표 | 값 |\n|---|---|\n| 매출 | 142억 |\n| 고객 | 1,280 |',
      x: 720, y: 220, w: 464, h: 200, z: 3, style: { fontSize: 16 },
    },
    {
      id: 'b4', kind: 'shape', md: '', x: 96, y: 520, w: 300, h: 120, z: 4,
      style: { fill: '#dbeafe', radius: 8 },
    },
    {
      id: 'b5', kind: 'image', md: '![없는 이미지](../assets/missing.png)',
      x: 460, y: 520, w: 300, h: 120, z: 5, style: {},
    },
  ];

  await call(`/api/projects/${encodeURIComponent(folder)}`, {
    method: 'PUT',
    body: JSON.stringify({ manifest: { title: '내보내기 덱' }, slides: created.slides }),
  });

  const { buffer, contentType, disposition } = await download(`/api/projects/${encodeURIComponent(folder)}/export/pptx`);
  const file = path.join(tmp, 'deck.pptx');
  await fs.writeFile(file, buffer);

  check('zip 시그니처를 가진 파일', buffer.slice(0, 2).toString() === 'PK', buffer.slice(0, 8).toString('hex'));
  check('presentationml MIME 타입', contentType.includes('presentationml'), contentType);
  check('파일 이름이 헤더에 있음', /filename\*=UTF-8/.test(disposition), disposition);
  check('파일 크기가 유의미함', buffer.length > 8000, `${buffer.length} bytes`);

  const { entries, xml } = await readZip(file);
  check('필수 파트 존재', entries.includes('[Content_Types].xml') && entries.some((e) => e === 'ppt/presentation.xml'),
    entries.slice(0, 8).join(', '));
  check('슬라이드 파트 존재', entries.some((e) => /^ppt\/slides\/slide1\.xml$/.test(e)), entries.filter((e) => e.includes('slide')).slice(0, 5).join(', '));
  check('발표자 노트 파트 존재', entries.some((e) => /notesSlide1\.xml$/.test(e)));
  check('제목 텍스트가 XML에 포함됨', xml.includes('분기 실적 요약'));
  check('굵게 표시한 런이 b="1"로 나감', /b="1"[^>]*\/>\s*<a:t>142억<\/a:t>|<a:t>142억<\/a:t>/.test(xml));
  check('목록 항목 텍스트 포함', xml.includes('신규 고객 1,280개사'));
  check('표 내용 포함', xml.includes('<a:tbl>') && xml.includes('142억'));
  check('발표자 노트 텍스트 포함', xml.includes('이 슬라이드의 발표자 노트'));
  check('찾을 수 없는 이미지는 대체 텍스트로', xml.includes('없는 이미지') || xml.includes('찾을 수 없음'));

  await call(`/api/projects/${encodeURIComponent(folder)}`, { method: 'DELETE' });
}

/* -------------------------------------------------------------------- doc */

console.log('\n■ Doc → .docx');
{
  const created = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'doc', title: '내보내기 문서', sample: false }),
  });
  const folder = created.folder;

  created.sections[0].blocks = [
    { id: 'h1', md: '# 제목 하나', type: 'heading', override: null },
    { id: 'p1', md: '본문에 **굵게**와 *기울임*, `코드`가 섞여 있습니다.', type: 'paragraph', override: null },
    { id: 'p2', md: '가운데 정렬한 문단입니다.', type: 'paragraph', override: { align: 'center', style: { color: '#4f46e5' } } },
    { id: 'h2', md: '## 소제목', type: 'heading', override: null },
    { id: 'l1', md: '- 첫째 항목\n- 둘째 항목', type: 'list', override: null },
    { id: 'l2', md: '1. 번호 하나\n2. 번호 둘', type: 'list', override: null },
    { id: 'q1', md: '> 인용된 문장', type: 'quote', override: null },
    { id: 't1', md: '| 열A | 열B |\n|---|---|\n| 1 | 2 |', type: 'table', override: null },
    { id: 'c1', md: '```js\nconst x = 1;\n```', type: 'code', override: null },
    { id: 'hr', md: '---', type: 'hr', override: null },
    { id: 'in', md: '들여쓴 문단', type: 'paragraph', override: { indent: 48 } },
  ];

  await call(`/api/projects/${encodeURIComponent(folder)}`, {
    method: 'PUT',
    body: JSON.stringify({ manifest: { title: '내보내기 문서' }, sections: created.sections }),
  });

  const { buffer, contentType } = await download(`/api/projects/${encodeURIComponent(folder)}/export/docx`);
  const file = path.join(tmp, 'doc.docx');
  await fs.writeFile(file, buffer);

  check('zip 시그니처를 가진 파일', buffer.slice(0, 2).toString() === 'PK');
  check('wordprocessingml MIME 타입', contentType.includes('wordprocessingml'), contentType);

  const { entries, xml } = await readZip(file);
  check('필수 파트 존재', entries.includes('word/document.xml') && entries.includes('[Content_Types].xml'),
    entries.slice(0, 8).join(', '));
  check('제목 텍스트 포함', xml.includes('제목 하나'));
  check('제목이 Heading 스타일로 나감', /w:pStyle w:val="Heading1"/.test(xml), sampleAround(xml, '제목 하나'));
  check('굵게 런이 <w:b/>로 나감', /<w:b\b/.test(xml));
  check('기울임 런이 <w:i/>로 나감', /<w:i\b/.test(xml));
  check('가운데 정렬이 반영됨', /w:jc w:val="center"/.test(xml));
  check('색 지정이 반영됨', /w:color w:val="4F46E5"/i.test(xml));
  check('글머리 목록 항목 포함', xml.includes('첫째 항목'));
  check('번호 목록이 numbering을 참조', /w:numPr/.test(xml) && entries.includes('word/numbering.xml'));
  check('인용문 포함', xml.includes('인용된 문장'));
  check('표가 <w:tbl>로 나감', xml.includes('<w:tbl>') && xml.includes('열A'));
  check('코드 블록 내용 포함', xml.includes('const x = 1;'));
  check('들여쓰기가 반영됨', /w:ind[^>]*w:left="720"/.test(xml), sampleAround(xml, '들여쓴 문단'));

  await call(`/api/projects/${encodeURIComponent(folder)}`, { method: 'DELETE' });
}

/* ------------------------------------------------------------------- grid */

console.log('\n■ Grid → .xlsx / .csv');
{
  const created = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'grid', title: '내보내기 시트', sample: true }),
  });
  const folder = created.folder;

  const sheet = created.sheets[0];
  sheet.cells.F1 = { v: '비중', t: 's', style: { bold: true, bg: '#e8f5ee' } };
  sheet.cells.F2 = { f: '=D2/$D$5', t: 'n', fmt: '0.0%' };
  sheet.cells.G1 = { v: '메모', t: 's', style: { border: { t: true, r: true, b: true, l: true } } };
  sheet.merges = ['A7:C7'];
  sheet.cells.A7 = { v: '병합된 제목', t: 's', style: { align: 'center', bold: true } };
  sheet.names = { 분기표: 'A1:F5' };
  sheet.colWidths = { A: 210 };

  await call(`/api/projects/${encodeURIComponent(folder)}`, {
    method: 'PUT',
    body: JSON.stringify({ manifest: { title: '내보내기 시트' }, sheets: created.sheets }),
  });

  const { buffer, contentType } = await download(`/api/projects/${encodeURIComponent(folder)}/export/xlsx`);
  const file = path.join(tmp, 'grid.xlsx');
  await fs.writeFile(file, buffer);

  check('zip 시그니처를 가진 파일', buffer.slice(0, 2).toString() === 'PK');
  check('spreadsheetml MIME 타입', contentType.includes('spreadsheetml'), contentType);

  const { entries, xml } = await readZip(file);
  check('필수 파트 존재', entries.includes('xl/workbook.xml') && entries.some((e) => /^xl\/worksheets\/sheet1\.xml$/.test(e)),
    entries.slice(0, 8).join(', '));
  check('수식이 값이 아니라 수식으로 나감', /<f>B2\+C2<\/f>/.test(xml), sampleAround(xml, '<f>'));
  check('절대 참조 수식도 보존됨', /<f>D2\/\$D\$5<\/f>/.test(xml));
  check('병합 정보가 mergeCell로 나감', /<mergeCell ref="A7:C7"\/>/.test(xml), sampleAround(xml, 'mergeCell'));
  check('틀 고정이 pane으로 나감', /<pane[^>]*state="frozen"/.test(xml), sampleAround(xml, 'pane'));
  check('이름 정의가 definedName으로 나감', /definedName name="분기표"/.test(xml), sampleAround(xml, 'definedName'));
  check('숫자 서식이 numFmt로 나감', /numFmt/.test(xml));
  check('열 너비가 지정됨', /<col\b[^>]*width=/.test(xml), sampleAround(xml, '<col '));
  check('텍스트가 sharedStrings 또는 inline으로 존재', entries.includes('xl/sharedStrings.xml') || /<is>/.test(xml));

  const csv = await download(`/api/projects/${encodeURIComponent(folder)}/export/csv`);
  const text = csv.buffer.toString('utf8');
  check('CSV MIME 타입', csv.contentType.includes('text/csv'), csv.contentType);
  check('CSV에 BOM이 있음 (Excel 한글)', csv.buffer.slice(0, 3).toString('hex') === 'efbbbf', csv.buffer.slice(0, 4).toString('hex'));
  check('CSV가 계산된 값을 담음', text.includes('2,550'), text.split('\r\n').slice(0, 3).join(' / '));
  check('CSV 머리글 행', text.includes('항목'));

  const digest = await download(`/api/projects/${encodeURIComponent(folder)}/digest`);
  check('AI.md 다이제스트를 내려받을 수 있음', digest.buffer.toString('utf8').includes('# 내보내기 시트'));

  await call(`/api/projects/${encodeURIComponent(folder)}`, { method: 'DELETE' });
}

/* ------------------------------------------------------------------ charts */

console.log('\n■ 차트 내보내기');
{
  const chartMd = [
    '```chart',
    JSON.stringify({
      type: 'column',
      title: '분기별 매출',
      labels: ['1분기', '2분기', '3분기'],
      series: [
        { name: '2025', values: [98, 114, 142] },
        { name: '2026', values: [120, 138, 171] },
      ],
    }, null, 2),
    '```',
  ].join('\n');

  // Deck: a real, editable PowerPoint chart part.
  const deck = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'deck', title: '차트 덱', sample: false }),
  });
  deck.slides[0].blocks = [
    { id: 'c1', kind: 'chart', md: chartMd, x: 96, y: 96, w: 900, h: 480, z: 1, style: {} },
  ];
  await call(`/api/projects/${encodeURIComponent(deck.folder)}`, {
    method: 'PUT',
    body: JSON.stringify({ manifest: { title: '차트 덱' }, slides: deck.slides }),
  });

  const pptx = await download(`/api/projects/${encodeURIComponent(deck.folder)}/export/pptx`);
  const pptxFile = path.join(tmp, 'chart.pptx');
  await fs.writeFile(pptxFile, pptx.buffer);
  const pptxZip = await readZip(pptxFile);

  check('pptx에 차트 파트가 생성됨', pptxZip.entries.some((e) => /^ppt\/charts\/chart\d+\.xml$/.test(e)),
    pptxZip.entries.filter((e) => e.includes('chart')).slice(0, 6).join(', '));
  check('차트가 그림이 아니라 데이터를 담음', pptxZip.xml.includes('분기별 매출') && pptxZip.xml.includes('<c:v>142</c:v>'),
    sampleAround(pptxZip.xml, '142'));
  check('두 계열 이름이 모두 들어감', pptxZip.xml.includes('2025') && pptxZip.xml.includes('2026'));
  check('범주 레이블이 들어감', pptxZip.xml.includes('1분기') && pptxZip.xml.includes('3분기'));
  check('막대 방향이 세로(col)로 지정됨', /<c:barDir val="col"\/>/.test(pptxZip.xml), sampleAround(pptxZip.xml, 'barDir'));

  const deckDigest = (await download(`/api/projects/${encodeURIComponent(deck.folder)}/digest`)).buffer.toString('utf8');
  check('AI.md가 차트를 표로 설명함', /\| 구분 \| 2025 \| 2026 \|/.test(deckDigest), deckDigest.slice(0, 400));
  check('AI.md가 차트 종류를 자연어로 씀', /세로 막대 차트/.test(deckDigest));
  check('AI.md에 차트 JSON이 그대로 덤프되지 않음', !deckDigest.includes('"labels"'), 'raw spec leaked');

  await call(`/api/projects/${encodeURIComponent(deck.folder)}`, { method: 'DELETE' });

  // Doc: the data table plus a caption, since .docx has no chart primitive.
  const doc = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'doc', title: '차트 문서', sample: false }),
  });
  doc.sections[0].blocks = [
    { id: 'h', md: '# 실적', type: 'heading', override: null },
    { id: 'c', md: chartMd, type: 'chart', override: null },
  ];
  await call(`/api/projects/${encodeURIComponent(doc.folder)}`, {
    method: 'PUT',
    body: JSON.stringify({ manifest: { title: '차트 문서' }, sections: doc.sections }),
  });

  const docx = await download(`/api/projects/${encodeURIComponent(doc.folder)}/export/docx`);
  const docxFile = path.join(tmp, 'chart.docx');
  await fs.writeFile(docxFile, docx.buffer);
  const docxZip = await readZip(docxFile);

  check('docx가 차트를 표로 내보냄', docxZip.xml.includes('<w:tbl>') && docxZip.xml.includes('142'),
    sampleAround(docxZip.xml, '142'));
  check('docx에 차트 제목이 들어감', docxZip.xml.includes('분기별 매출'));
  check('docx 캡션이 차트 종류를 설명함', /세로 막대/.test(docxZip.xml), sampleAround(docxZip.xml, '표 데이터'));
  check('docx에 원본 JSON이 새지 않음', !docxZip.xml.includes('"series"'));

  await call(`/api/projects/${encodeURIComponent(doc.folder)}`, { method: 'DELETE' });

  // Grid: a range-backed chart, with its numbers written out for Excel.
  const grid = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'grid', title: '차트 시트', sample: true }),
  });
  grid.sheets[0].charts = [
    { id: 'ch1', x: 60, y: 60, w: 480, h: 300, spec: { type: 'bar', title: '항목별 합계', range: 'A1:D5' } },
  ];
  await call(`/api/projects/${encodeURIComponent(grid.folder)}`, {
    method: 'PUT',
    body: JSON.stringify({ manifest: { title: '차트 시트' }, sheets: grid.sheets }),
  });

  const reloadedGrid = await call(`/api/projects/${encodeURIComponent(grid.folder)}`);
  check('시트 차트가 저장되고 다시 읽힘', reloadedGrid.sheets[0].charts?.[0]?.spec?.range === 'A1:D5',
    JSON.stringify(reloadedGrid.sheets[0].charts));

  const sheetMd = await read(grid.folder, reloadedGrid.manifest.sheets[0].md);
  check('시트 md가 차트 절을 담음', sheetMd.includes('### 차트') && sheetMd.includes('가로 막대'),
    sheetMd.slice(sheetMd.indexOf('### 차트'), sheetMd.indexOf('### 차트') + 300));
  check('시트 md의 차트가 범위에서 값을 끌어옴', /제품 A/.test(sheetMd.slice(sheetMd.indexOf('### 차트'))),
    sheetMd.slice(sheetMd.indexOf('### 차트'), sheetMd.indexOf('### 차트') + 400));

  const xlsx = await download(`/api/projects/${encodeURIComponent(grid.folder)}/export/xlsx`);
  const xlsxFile = path.join(tmp, 'chart.xlsx');
  await fs.writeFile(xlsxFile, xlsx.buffer);
  const xlsxZip = await readZip(xlsxFile);
  check('xlsx가 차트 데이터를 블록으로 기록함', xlsxZip.xml.includes('항목별 합계') || (await hasSharedString(xlsxFile, '항목별 합계')),
    'chart data block missing');

  await call(`/api/projects/${encodeURIComponent(grid.folder)}`, { method: 'DELETE' });
}

/* ------------------------------------------------------------------ assets */

console.log('\n■ 이미지 자산');
{
  const created = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'deck', title: '이미지 덱', sample: false }),
  });
  const folder = encodeURIComponent(created.folder);

  // A 1x1 transparent PNG.
  const png =
    'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==';

  const saved = await call(`/api/projects/${folder}/assets`, {
    method: 'POST',
    body: JSON.stringify({ name: '내 그림.png', dataUrl: png }),
  });
  check('업로드가 assets/ 아래에 저장됨', saved.path.startsWith('assets/') && saved.path.endsWith('.png'), saved.path);
  check('한글 파일 이름이 안전하게 정리됨', /^assets\/[^/]+\.png$/.test(saved.path), saved.path);

  const listed = await call(`/api/projects/${folder}/assets`);
  check('자산 목록에 나타남', listed.assets.some((a) => a.path === saved.path), JSON.stringify(listed.assets));

  const served = await download(`/api/projects/${folder}/asset?path=${encodeURIComponent(saved.path)}`);
  check('이미지가 그대로 제공됨', served.buffer.slice(1, 4).toString() === 'PNG', served.buffer.slice(0, 8).toString('hex'));
  check('이미지 MIME 타입', served.contentType.includes('image/png'), served.contentType);

  // Same name again must not overwrite.
  const second = await call(`/api/projects/${folder}/assets`, {
    method: 'POST',
    body: JSON.stringify({ name: '내 그림.png', dataUrl: png }),
  });
  check('같은 이름은 덮어쓰지 않고 새 파일이 됨', second.path !== saved.path, `${saved.path} vs ${second.path}`);

  // The deck should embed it when exporting.
  created.slides[0].blocks = [
    { id: 'i1', kind: 'image', md: `![업로드한 그림](../${saved.path})`, x: 96, y: 96, w: 400, h: 300, z: 1, style: {} },
  ];
  await call(`/api/projects/${folder}`, {
    method: 'PUT',
    body: JSON.stringify({ manifest: { title: '이미지 덱' }, slides: created.slides }),
  });
  const pptx = await download(`/api/projects/${folder}/export/pptx`);
  const file = path.join(tmp, 'image.pptx');
  await fs.writeFile(file, pptx.buffer);
  const zip = await readZip(file);
  check('pptx에 이미지 미디어가 포함됨', zip.entries.some((e) => /^ppt\/media\//.test(e)),
    zip.entries.filter((e) => e.includes('media')).join(', '));

  for (const [attempt, why] of [
    ['../../../etc/passwd', '경로 탈출'],
    ['manifest.json', 'assets 밖의 파일'],
  ]) {
    let rejected = false;
    try {
      await download(`/api/projects/${folder}/asset?path=${encodeURIComponent(attempt)}`);
    } catch (e) {
      rejected = /400|404/.test(e.message);
    }
    check(`${why} 요청 거부: ${attempt}`, rejected);
  }

  let badType = false;
  try {
    await call(`/api/projects/${folder}/assets`, {
      method: 'POST',
      body: JSON.stringify({ name: 'x.exe', dataUrl: 'data:application/x-msdownload;base64,AAAA' }),
    });
  } catch (e) {
    badType = /400/.test(e.message);
  }
  check('이미지가 아닌 형식은 거부', badType);

  await call(`/api/projects/${decodeURIComponent(folder)}`, { method: 'DELETE' });
}

/* --------------------------------------------------------------- rejections */

console.log('\n■ 잘못된 내보내기 요청');
{
  const created = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'doc', title: '거부 확인', sample: false }),
  });
  const folder = encodeURIComponent(created.folder);

  for (const [ext, why] of [['pptx', '문서를 pptx로'], ['zzz', '알 수 없는 형식']]) {
    let rejected = false;
    try {
      await download(`/api/projects/${folder}/export/${ext}`);
    } catch (e) {
      rejected = /400/.test(e.message);
    }
    check(`${why} 요청은 400으로 거부`, rejected);
  }
  await call(`/api/projects/${folder}`, { method: 'DELETE' });
}

await fs.rm(tmp, { recursive: true, force: true });

console.log(`\n${failures === 0 ? '통과' : '실패'}: ${checks - failures}/${checks} 검사 성공`);
process.exit(failures === 0 ? 0 : 1);

/** Look for a string inside xl/sharedStrings.xml, where Excel puts text. */
async function hasSharedString(file, needle) {
  try {
    const { stdout } = await run('unzip', ['-oq', file, '-d', `${file}.ss`]);
    const text = await fs.readFile(path.join(`${file}.ss`, 'xl/sharedStrings.xml'), 'utf8');
    return text.includes(needle);
  } catch {
    return false;
  }
}

function sampleAround(xml, needle) {
  const at = xml.indexOf(needle);
  return at < 0 ? `"${needle}" 없음` : xml.slice(Math.max(0, at - 120), at + 160);
}
