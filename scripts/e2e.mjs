/**
 * End-to-end check against a running server: create each project type, edit it
 * through the API the way the editor does, then verify what landed on disk.
 *
 * Run with: node scripts/e2e.mjs
 */
import { promises as fs } from 'node:fs';
import path from 'node:path';

const BASE = process.env.AI_STUDIO_API ?? 'http://localhost:5177';
let failures = 0;
let checks = 0;

function check(label, condition, detail) {
  checks++;
  if (condition) {
    console.log(`  ✓ ${label}`);
  } else {
    failures++;
    console.log(`  ✗ ${label}${detail ? `\n      ${detail}` : ''}`);
  }
}

async function call(path, options) {
  const res = await fetch(`${BASE}${path}`, {
    headers: options?.body ? { 'Content-Type': 'application/json' } : undefined,
    ...options,
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${options?.method ?? 'GET'} ${path} → ${res.status}: ${text.slice(0, 300)}`);
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

const { workspace } = await call('/api/health');
const read = (folder, rel) => fs.readFile(path.join(workspace, folder, rel), 'utf8');
const readJson = async (folder, rel) => JSON.parse(await read(folder, rel));

/* ------------------------------------------------------------------- deck */

console.log('\n■ Deck');
{
  const created = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'deck', title: 'E2E 덱' }),
  });
  const folder = created.folder;

  const slide = created.slides[0];
  slide.blocks[0].md = '# 수정된 제목';
  slide.blocks[0].x = 200;
  slide.blocks[0].y = 100;
  slide.blocks[0].w = 600; // must fit the canvas, or clamping moves x
  slide.notes = '이 슬라이드에서 강조할 점';
  slide.blocks.push({
    id: 'newblk',
    kind: 'text',
    md: '- 새 항목 A\n- 새 항목 B',
    x: 96,
    y: 460,
    w: 600,
    h: 140,
    z: 9,
    style: { fontSize: 18 },
  });

  const saved = await call(`/api/projects/${encodeURIComponent(folder)}`, {
    method: 'PUT',
    body: JSON.stringify({ manifest: { title: 'E2E 덱' }, slides: created.slides }),
  });

  const files = (await call(`/api/projects/${encodeURIComponent(folder)}/files`)).files.map((f) => f.path);
  check('md + layout.json pair exists per slide',
    files.filter((f) => f.endsWith('.md') && f.startsWith('slides/')).length === created.slides.length &&
    files.filter((f) => f.endsWith('.layout.json')).length === created.slides.length,
    files.join(', '));
  check('AI.md and manifest.json exist', files.includes('AI.md') && files.includes('manifest.json'));

  const mdPath = saved.manifest.slides[0].md;
  const md = await read(folder, mdPath);
  const layout = await readJson(folder, saved.manifest.slides[0].json);

  check('edited text is in the markdown', md.includes('# 수정된 제목'), md.slice(0, 200));
  check('speaker notes are in the frontmatter', md.includes('이 슬라이드에서 강조할 점'));
  check('the added block is in the markdown', md.includes('새 항목 A'));
  check('coordinates are NOT in the markdown', !/"x"|\bx:\s*\d/.test(md));
  check('coordinates ARE in the layout json',
    Object.values(layout.blocks).some((b) => b.x === 200 && b.y === 100),
    JSON.stringify(layout.blocks).slice(0, 200));
  check('text is NOT in the layout json', !JSON.stringify(layout).includes('수정된 제목'));

  const digest = await read(folder, 'AI.md');
  check('AI.md describes position in words', /상단|중앙|하단|정중앙/.test(digest));
  check('AI.md contains the slide text', digest.includes('수정된 제목'));
  check('AI.md contains the speaker notes', digest.includes('이 슬라이드에서 강조할 점'));
  check('AI.md has no raw pixel coordinates', !/x=\d+|"x":/.test(digest));

  const reloaded = await call(`/api/projects/${encodeURIComponent(folder)}`);
  const back = reloaded.slides[0];
  check('reload preserves text', back.blocks.some((b) => b.md.includes('수정된 제목')));
  check('reload preserves geometry', back.blocks.some((b) => b.x === 200 && b.y === 100));
  check('reload preserves notes', back.notes.includes('강조할 점'));
  check('reload preserves the added block', back.blocks.some((b) => b.md.includes('새 항목 A')));
  check('block count survived the round trip', back.blocks.length === slide.blocks.length,
    `${back.blocks.length} vs ${slide.blocks.length}`);

  // A hand-edited markdown file must still open, with the new block picked up.
  await fs.appendFile(
    path.join(workspace, folder, mdPath),
    '\n<!-- block:byhand -->\n손으로 추가한 블록\n',
    'utf8'
  );
  const afterHandEdit = await call(`/api/projects/${encodeURIComponent(folder)}`);
  const handBlock = afterHandEdit.slides[0].blocks.find((b) => b.id === 'byhand');
  check('a hand-edited markdown block is picked up', !!handBlock);
  check('the hand-added block got auto geometry', handBlock && handBlock.w > 0 && handBlock.h > 0,
    JSON.stringify(handBlock));

  await call(`/api/projects/${encodeURIComponent(folder)}`, { method: 'DELETE' });
}

/* -------------------------------------------------------------------- doc */

console.log('\n■ Doc');
{
  const created = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'doc', title: 'E2E 문서' }),
  });
  const folder = created.folder;

  const section = created.sections[0];
  section.blocks = [
    { id: 'h1', md: '# 서론', type: 'heading', override: null },
    { id: 'p1', md: '기본 서식 문단입니다.', type: 'paragraph', override: null },
    { id: 'p2', md: '> 가운데 정렬한 인용문', type: 'quote', override: { align: 'center', style: { italic: true } } },
    { id: 'h2', md: '## 배경', type: 'heading', override: null },
    { id: 'l1', md: '- 항목 하나\n- 항목 둘', type: 'list', override: null },
  ];

  const saved = await call(`/api/projects/${encodeURIComponent(folder)}`, {
    method: 'PUT',
    body: JSON.stringify({ manifest: { title: 'E2E 문서' }, sections: created.sections }),
  });

  const md = await read(folder, saved.manifest.sections[0].md);
  const meta = await readJson(folder, saved.manifest.sections[0].json);

  check('markdown is clean where formatting is default',
    md.includes('# 서론') && md.includes('기본 서식 문단입니다.') &&
    !md.includes('<!-- block:h1 -->') && !md.includes('<!-- block:p1 -->'),
    md);
  check('only the styled paragraph carries an anchor',
    (md.match(/<!-- block:/g) ?? []).length === 1 && md.includes('<!-- block:p2 -->'),
    md);
  check('meta.json holds exactly one override', Object.keys(meta.blocks).length === 1 && !!meta.blocks.p2,
    JSON.stringify(meta.blocks));
  check('the override records the alignment', meta.blocks.p2.align === 'center');
  check('meta.json carries a derived outline',
    meta.outline.length === 2 && meta.outline[0].text === '서론' && meta.outline[1].level === 2,
    JSON.stringify(meta.outline));
  check('meta.json carries word stats', meta.stats.words > 0);

  const reloaded = await call(`/api/projects/${encodeURIComponent(folder)}`);
  const blocks = reloaded.sections[0].blocks;
  check('reload recovers all five blocks', blocks.length === 5, `got ${blocks.length}`);
  check('reload recovers the override',
    blocks.find((b) => b.md.includes('인용문'))?.override?.align === 'center',
    JSON.stringify(blocks.map((b) => ({ md: b.md.slice(0, 12), o: b.override }))));
  check('reload keeps default blocks override-free',
    blocks.filter((b) => b.override).length === 1);

  const digest = await read(folder, 'AI.md');
  check('AI.md has a table of contents', digest.includes('## 목차') && digest.includes('- 서론'));
  check('AI.md contains the body text', digest.includes('기본 서식 문단입니다.'));

  await call(`/api/projects/${encodeURIComponent(folder)}`, { method: 'DELETE' });
}

/* ------------------------------------------------------------------- grid */

console.log('\n■ Grid');
{
  const created = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'grid', title: 'E2E 시트' }),
  });
  const folder = created.folder;

  const sheet = created.sheets[0];
  sheet.cells.F1 = { v: '비율', t: 's', style: { bold: true } };
  sheet.cells.F2 = { f: '=D2/D$5', t: 'n', fmt: '0.0%' };
  sheet.cells.H1 = { f: '=SUM(B2:C4)', t: 'n', fmt: '#,##0' };
  sheet.cells.H2 = { f: '=1/0', t: 'n' };
  sheet.names = { 매출데이터: 'A1:D5' };

  const saved = await call(`/api/projects/${encodeURIComponent(folder)}`, {
    method: 'PUT',
    body: JSON.stringify({ manifest: { title: 'E2E 시트' }, sheets: created.sheets }),
  });

  const md = await read(folder, saved.manifest.sheets[0].md);
  const cells = await readJson(folder, saved.manifest.sheets[0].json);

  check('formulas are stored in the json', cells.cells.H1.f === '=SUM(B2:C4)');
  check('the server computed and cached the value', cells.cells.H1.v === 6180,
    `H1.v = ${cells.cells.H1.v}`);
  check('errors are cached as error cells', cells.cells.H2.v === '#DIV/0!' && cells.cells.H2.t === 'e',
    JSON.stringify(cells.cells.H2));
  check('named ranges are stored', cells.names['매출데이터'] === 'A1:D5');

  const tablePart = md.slice(0, md.indexOf('### 수식'));
  check('the markdown table has column letters', /\|\s+\|\s*A\s*\|\s*B\s*\|/.test(tablePart), tablePart.slice(0, 200));
  check('the markdown table has row numbers', /\|\s*\*\*2\*\*\s*\|/.test(tablePart));
  check('the markdown table shows values, not formulas', !tablePart.includes('=SUM('), tablePart.slice(0, 400));
  check('the markdown table shows formatted numbers', tablePart.includes('1,200'));
  check('the formula appendix pairs expression with result',
    /`H1` = `=SUM\(B2:C4\)` → 6,180/.test(md), md.slice(md.indexOf('### 수식'), md.indexOf('### 수식') + 400));
  check('the markdown lists named ranges', md.includes('`매출데이터` → `A1:D5`'));
  check('the markdown has a column summary', md.includes('### 열 요약'));

  const reloaded = await call(`/api/projects/${encodeURIComponent(folder)}`);
  const back = reloaded.sheets[0];
  check('reload preserves formulas', back.cells.H1.f === '=SUM(B2:C4)');
  check('reload preserves computed values', back.cells.H1.v === 6180);
  check('reload preserves cell styles', back.cells.F1.style.bold === true);
  check('reload preserves number formats', back.cells.F2.fmt === '0.0%');
  check('reload preserves named ranges', back.names['매출데이터'] === 'A1:D5');

  const digest = await read(folder, 'AI.md');
  check('AI.md turns rows into key=value records', /2행: 항목=제품 A/.test(digest),
    digest.slice(digest.indexOf('### 데이터'), digest.indexOf('### 데이터') + 300));
  check('AI.md lists formulas with results', /`H1`: `=SUM\(B2:C4\)` → \*\*6,180\*\*/.test(digest));
  check('AI.md describes the columns', digest.includes('### 열 구성'));

  const recalc = await call('/api/recalc', {
    method: 'POST',
    body: JSON.stringify({ cells: { A1: { v: 10 }, A2: { v: 32 }, A3: { f: '=SUM(A1:A2)' } } }),
  });
  check('the /api/recalc endpoint computes formulas', recalc.cells.A3.v === 42, JSON.stringify(recalc.cells.A3));

  await call(`/api/projects/${encodeURIComponent(folder)}`, { method: 'DELETE' });
}

/* ------------------------------------------------------------------ rename */

console.log('\n■ 제목 변경 → 폴더 이름 추적');
{
  const created = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'doc', title: '이전 제목' }),
  });
  check('folder name follows the creation title', created.folder.startsWith('이전-제목'), created.folder);

  const renamed = await call(`/api/projects/${encodeURIComponent(created.folder)}`, {
    method: 'PATCH',
    body: JSON.stringify({ title: '새로운 제목' }),
  });
  check('rename returns the new folder', renamed.folder.startsWith('새로운-제목'), renamed.folder);
  check('rename updates the manifest title', renamed.manifest.title === '새로운 제목');

  const listed = (await call('/api/projects')).projects.map((p) => p.folder);
  check('the old folder is gone', !listed.includes(created.folder), listed.join(', '));
  check('the new folder is listed', listed.includes(renamed.folder));

  const digest = await read(renamed.folder, 'AI.md');
  check('AI.md was regenerated with the new title', digest.includes('# 새로운 제목'));

  let blankRejected = false;
  try {
    await call(`/api/projects/${encodeURIComponent(renamed.folder)}`, {
      method: 'PATCH',
      body: JSON.stringify({ title: '   ' }),
    });
  } catch (e) {
    blankRejected = e.message.includes('400');
  }
  check('a blank title is rejected', blankRejected);

  await call(`/api/projects/${encodeURIComponent(renamed.folder)}`, { method: 'DELETE' });
}

/* ---------------------------------------------------------------- import */

console.log('\n■ 기존 Office 파일 열기');
{
  // Export one of this project's own documents, then open the result. Anything
  // the writer emits and the reader drops shows up here immediately.
  const created = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'grid', title: '내보내고 다시 열기' }),
  });
  const url = `${BASE}/api/projects/${encodeURIComponent(created.folder)}/export/xlsx`;
  const bytes = Buffer.from(await (await fetch(url)).arrayBuffer());
  check('내보낸 xlsx가 비어 있지 않음', bytes.length > 2000, `${bytes.length} bytes`);

  const imported = await call('/api/import', {
    method: 'POST',
    body: JSON.stringify({ name: '가져온 예산.xlsx', data: bytes.toString('base64') }),
  });
  check('가져오기가 폴더를 만듦', imported.folder?.endsWith('.aigrid'), imported.folder);
  check('제목이 파일명에서 옴', imported.project.manifest.title === '가져온 예산');

  const sheet = imported.project.sheets[0];
  check('수식이 수식으로 들어옴', sheet.cells?.D2?.f === '=B2+C2', JSON.stringify(sheet.cells?.D2));
  check('표시 형식이 유지됨', sheet.cells?.B2?.fmt === '#,##0');
  check('셀 스타일이 유지됨', sheet.cells?.A1?.style?.bold === true, JSON.stringify(sheet.cells?.A1));
  check('틀 고정이 유지됨', sheet.frozen?.rows === 1 && sheet.frozen?.cols === 1);

  // And the folder on disk is a normal project: md, json and AI.md.
  const files = (await call(`/api/projects/${encodeURIComponent(imported.folder)}/files`)).files.map((f) => f.path);
  check('디스크에 AI.md가 있음', files.includes('AI.md'), files.join(', '));
  check('디스크에 md/json 쌍이 있음',
    files.some((f) => f.endsWith('.md') && f.startsWith('sheets/')) &&
    files.some((f) => f.endsWith('.cells.json')));

  const digest = await read(imported.folder, 'AI.md');
  check('AI.md가 가져온 내용을 담음', digest.includes('# 가져온 예산') && digest.includes('### 수식과 계산 결과'));

  await call(`/api/projects/${encodeURIComponent(imported.folder)}`, { method: 'DELETE' });
  await call(`/api/projects/${encodeURIComponent(created.folder)}`, { method: 'DELETE' });
}

console.log('\n■ 가져온 문서가 원본대로 보이는지');
{
  // A Word document whose author turned the page and set a heading size. Both
  // are things the editor draws differently from its own defaults, so both have
  // to arrive as stated rather than as the app would have chosen.
  const created = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'doc', title: '가로 문서' }),
  });
  const section = created.sections[0];
  await call(`/api/projects/${encodeURIComponent(created.folder)}`, {
    method: 'PUT',
    body: JSON.stringify({
      sections: [{ ...section, page: { size: 'A4', width: 1123, height: 794, margin: section.page.margin, columns: 1 } }],
    }),
  });

  const url = `${BASE}/api/projects/${encodeURIComponent(created.folder)}/export/docx`;
  const bytes = Buffer.from(await (await fetch(url)).arrayBuffer());
  const imported = await call('/api/import', {
    method: 'POST',
    body: JSON.stringify({ name: '가로 문서.docx', data: bytes.toString('base64') }),
  });
  const page = imported.project.sections[0].page;
  check('가로 방향이 유지됨', page.width === 1123 && page.height === 794, JSON.stringify(page));
  check('용지 이름은 여전히 A4', page.size === 'A4', page.size);
  // This file was written by the app, so it already names the app's own font and
  // there is nothing to substitute. A file from Word reports the swap instead —
  // that path is covered in the Rust tests, where the fonts can be dictated.
  check('직접 내보낸 파일은 글꼴 경고가 없음',
    !(imported.warnings ?? []).some((w) => w.includes('글꼴')),
    JSON.stringify(imported.warnings));

  await call(`/api/projects/${encodeURIComponent(imported.folder)}`, { method: 'DELETE' });
  await call(`/api/projects/${encodeURIComponent(created.folder)}`, { method: 'DELETE' });
}

console.log('\n■ 시트 사이 수식과 머리글이 저장을 견디는지');
{
  // The two things a real workbook and a real document are full of, through the
  // API and back off the disk: a cross-sheet formula and a page number.
  const grid = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'grid', title: '시트 사이 수식' }),
  });
  const first = grid.sheets[0];
  const second = { ...first, id: 'sh_two', name: '요약', cells: { A1: { f: `=${first.name}!B2*2` } }, charts: [] };
  await call(`/api/projects/${encodeURIComponent(grid.folder)}`, {
    method: 'PUT',
    body: JSON.stringify({ sheets: [first, second] }),
  });

  const reopened = await call(`/api/projects/${encodeURIComponent(grid.folder)}`);
  const summary = reopened.sheets.find((s) => s.name === '요약');
  check('시트 사이 수식이 그대로 저장됨', summary.cells.A1.f === `=${first.name}!B2*2`, summary.cells.A1.f);
  check('값이 다른 시트에서 계산됨', Number(summary.cells.A1.v) === Number(first.cells.B2.v) * 2,
    `${summary.cells.A1.v} vs ${first.cells.B2.v}`);

  const projection = await read(grid.folder, 'sheets/02-요약.md');
  check('md 투영에도 계산된 값이 들어감', /2,?\d{3}/.test(projection), projection.slice(0, 200));

  const doc = await call('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ type: 'doc', title: '페이지 번호' }),
  });
  const section = doc.sections[0];
  await call(`/api/projects/${encodeURIComponent(doc.folder)}`, {
    method: 'PUT',
    body: JSON.stringify({
      sections: [{ ...section, page: { ...section.page, footer: { center: '{PAGE} / {PAGES}' } } }],
    }),
  });
  const backDoc = await call(`/api/projects/${encodeURIComponent(doc.folder)}`);
  check('바닥글이 저장되고 다시 열림',
    backDoc.sections[0].page.footer?.center === '{PAGE} / {PAGES}',
    JSON.stringify(backDoc.sections[0].page));

  const url = `${BASE}/api/projects/${encodeURIComponent(doc.folder)}/export/docx`;
  const bytes = Buffer.from(await (await fetch(url)).arrayBuffer());
  const imported = await call('/api/import', {
    method: 'POST',
    body: JSON.stringify({ name: '페이지 번호.docx', data: bytes.toString('base64') }),
  });
  check('내보낸 docx의 바닥글이 다시 읽힘',
    imported.project.sections[0].page.footer?.center === '{PAGE} / {PAGES}',
    JSON.stringify(imported.project.sections[0].page.footer));

  for (const folder of [grid.folder, doc.folder, imported.folder]) {
    await call(`/api/projects/${encodeURIComponent(folder)}`, { method: 'DELETE' });
  }
}

console.log('\n■ 열 수 없는 파일');
{
  for (const [name, expect] of [
    ['old.ppt', /2007년 이전/],
    ['photo.png', /pptx/],
  ]) {
    let message = '';
    try {
      await call('/api/import', {
        method: 'POST',
        body: JSON.stringify({ name, data: Buffer.from('not office').toString('base64') }),
      });
    } catch (e) {
      message = e.message;
    }
    check(`${name}은 무엇을 해야 할지 알려줌`, expect.test(message), message);
  }
}

/* -------------------------------------------------------------- security */

console.log('\n■ 경로 보안');
{
  const attempts = ['../../../etc', '..%2f..%2fetc', 'not-a-project'];
  for (const attempt of attempts) {
    let rejected = false;
    try {
      await call(`/api/projects/${encodeURIComponent(attempt)}`);
    } catch (e) {
      rejected = /400|404/.test(e.message);
    }
    check(`traversal attempt rejected: ${attempt}`, rejected);
  }
}

console.log(`\n${failures === 0 ? '통과' : '실패'}: ${checks - failures}/${checks} 검사 성공`);
process.exit(failures === 0 ? 0 : 1);
