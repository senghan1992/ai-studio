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
