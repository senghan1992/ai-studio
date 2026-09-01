import test from 'node:test';
import assert from 'node:assert/strict';

import {
  parseFrontmatter, serializeFrontmatter,
  splitBlocks, joinBlocks, inferKind, blockLabel,
  splitMarkdownBlocks, countWords, plainText,
  positionPhrase, autoLayout, readingOrder,
  readSlide, writeSlide, makeSlide,
  readSection, writeSection, makeSection,
  readSheet, writeSheet, makeSheet, importMarkdownTable, renderSheetMarkdown,
  buildDigest,
} from '../src/browser.js';

/* ------------------------------------------------------------ frontmatter */

test('frontmatter round-trips', () => {
  const md = serializeFrontmatter({ id: 's1', title: '표지' }, '# 안녕');
  const { meta, body } = parseFrontmatter(md);
  assert.equal(meta.id, 's1');
  assert.equal(meta.title, '표지');
  assert.equal(body.trim(), '# 안녕');
});

test('missing frontmatter yields empty meta, not a throw', () => {
  const { meta, body } = parseFrontmatter('# 그냥 마크다운');
  assert.deepEqual(meta, {});
  assert.equal(body.trim(), '# 그냥 마크다운');
});

test('malformed frontmatter degrades gracefully', () => {
  const { meta, body } = parseFrontmatter('---\n: : bad yaml [\n---\n\nbody');
  assert.deepEqual(meta, {});
  assert.equal(body.trim(), 'body');
});

test('multiline values survive the round trip', () => {
  const notes = '첫 줄\n둘째 줄';
  const md = serializeFrontmatter({ notes }, 'x');
  assert.equal(parseFrontmatter(md).meta.notes.trim(), notes);
});

/* ----------------------------------------------------------------- blocks */

test('block markers split content', () => {
  const blocks = splitBlocks('<!-- block:b1 -->\n# A\n\n<!-- block:b2 -->\n- x\n- y');
  assert.equal(blocks.length, 2);
  assert.equal(blocks[0].id, 'b1');
  assert.equal(blocks[0].md, '# A');
  assert.equal(blocks[1].md, '- x\n- y');
});

test('markdown with no markers becomes one implicit block', () => {
  const blocks = splitBlocks('# 손으로 쓴 문서\n\n본문');
  assert.equal(blocks.length, 1);
  assert.equal(blocks[0].implicit, true);
  assert.match(blocks[0].md, /손으로 쓴 문서/);
});

test('a block marker inside a code fence stays content', () => {
  const src = '<!-- block:b1 -->\n```html\n<!-- block:fake -->\n```';
  const blocks = splitBlocks(src);
  assert.equal(blocks.length, 1);
  assert.match(blocks[0].md, /block:fake/);
});

test('duplicate ids in a hand-edited file do not collapse blocks', () => {
  const blocks = splitBlocks('<!-- block:dup -->\nA\n\n<!-- block:dup -->\nB');
  assert.equal(blocks.length, 2);
  assert.notEqual(blocks[0].id, blocks[1].id);
});

test('joinBlocks and splitBlocks are inverse', () => {
  const original = [
    { id: 'b1', md: '# 제목' },
    { id: 'b2', md: '- 항목 1\n- 항목 2' },
  ];
  const round = splitBlocks(joinBlocks(original));
  assert.deepEqual(
    round.map((b) => ({ id: b.id, md: b.md })),
    original
  );
});

test('inferKind recognises block content', () => {
  assert.equal(inferKind('# 제목'), 'text');
  assert.equal(inferKind('![alt](a.png)'), 'image');
  assert.equal(inferKind('| a | b |\n|---|---|\n| 1 | 2 |'), 'table');
  assert.equal(inferKind('```chart\n{}\n```'), 'chart');
});

test('blockLabel prefers the heading and strips markup', () => {
  assert.equal(blockLabel('## **굵은** 제목'), '굵은 제목');
  assert.equal(blockLabel('- 리스트 항목'), '리스트 항목');
  assert.equal(blockLabel(''), '');
});

/* ------------------------------------------------------- markdown blocks */

test('splitMarkdownBlocks segments a flow document', () => {
  const parts = splitMarkdownBlocks(
    '# 제목\n\n문단 하나.\n\n- a\n- b\n\n> 인용\n\n```js\nlet x = 1;\n\nlet y = 2;\n```'
  );
  assert.deepEqual(parts.map((p) => p.type), ['heading', 'paragraph', 'list', 'quote', 'code']);
  assert.match(parts[4].md, /let y = 2/, 'blank line inside a fence does not split it');
});

test('consecutive table rows stay one block', () => {
  const parts = splitMarkdownBlocks('| a | b |\n|---|---|\n| 1 | 2 |');
  assert.equal(parts.length, 1);
  assert.equal(parts[0].type, 'table');
});

test('countWords counts CJK characters individually', () => {
  assert.equal(countWords('hello world'), 2);
  assert.equal(countWords('한글 단어'), 4);
  assert.equal(countWords('mixed 한글 text'), 4);
  assert.equal(countWords(''), 0);
});

test('plainText strips markdown syntax', () => {
  assert.equal(plainText('## **굵게** 그리고 [링크](http://x)'), '굵게 그리고 링크');
});

/* --------------------------------------------------------------- geometry */

test('positionPhrase translates pixels into prose', () => {
  const canvas = { w: 1280, h: 720 };
  assert.match(positionPhrase({ x: 96, y: 40, w: 1088, h: 80 }, canvas), /상단 중앙.*전체 폭/);
  assert.match(positionPhrase({ x: 40, y: 600, w: 300, h: 80 }, canvas), /하단 좌측/);
  assert.equal(positionPhrase({ x: 440, y: 300, w: 400, h: 120 }, canvas), '정중앙, 좁은 폭');
});

test('autoLayout stacks blocks without overlap', () => {
  const canvas = { w: 1280, h: 720 };
  const boxes = [0, 1, 2].map((i) => autoLayout(i, 3, canvas));
  for (let i = 1; i < boxes.length; i++) {
    assert.ok(boxes[i].y >= boxes[i - 1].y + boxes[i - 1].h, `block ${i} clears block ${i - 1}`);
  }
  assert.ok(boxes[2].y + boxes[2].h <= canvas.h, 'last block fits on the canvas');
});

test('readingOrder sorts top-to-bottom then left-to-right', () => {
  const blocks = [
    { id: 'c', x: 700, y: 400 },
    { id: 'a', x: 100, y: 100 },
    { id: 'b', x: 700, y: 110 },
  ];
  assert.deepEqual(readingOrder(blocks).map((b) => b.id), ['a', 'b', 'c']);
});

/* ------------------------------------------------------------------- deck */

test('slide round-trips through the md + layout pair', () => {
  const slide = makeSlide('title', { title: '분기 리뷰' });
  slide.notes = '3분 안에 넘어간다';
  const { md, layout } = writeSlide(slide);
  const back = readSlide({ md, layout });

  assert.equal(back.title, slide.title);
  assert.equal(back.notes, slide.notes);
  assert.equal(back.layoutName, 'title');
  assert.equal(back.blocks.length, slide.blocks.length);
  back.blocks.forEach((b, i) => {
    assert.equal(b.md, slide.blocks[i].md, `block ${i} text`);
    assert.equal(b.x, slide.blocks[i].x, `block ${i} x`);
    assert.equal(b.y, slide.blocks[i].y, `block ${i} y`);
    assert.equal(b.kind, slide.blocks[i].kind);
  });
});

test('slide markdown holds the content and layout json holds the geometry', () => {
  const { md, layout } = writeSlide(makeSlide('title', { title: '분리 확인' }));
  assert.match(md, /# 분리 확인/, 'text lives in markdown');
  assert.ok(!/\bx:|"x"/.test(md), 'coordinates never leak into markdown');
  assert.ok(!JSON.stringify(layout).includes('분리 확인'), 'text never leaks into layout json');
  const firstBlock = Object.values(layout.blocks)[0];
  assert.equal(typeof firstBlock.x, 'number');
});

test('a slide written by hand with no layout json still opens', () => {
  const md = '---\nid: s9\ntitle: 수동\n---\n\n# 손으로 쓴 슬라이드\n\n본문 문단';
  const slide = readSlide({ md, layout: null });
  assert.equal(slide.title, '수동');
  assert.equal(slide.blocks.length, 1, 'no markers means one block');
  assert.ok(slide.blocks[0].w > 0 && slide.blocks[0].h > 0, 'auto layout gave it a size');
});

test('a block added to markdown by hand gets an auto layout', () => {
  const slide = makeSlide('title-content', { title: 'A' });
  const { md, layout } = writeSlide(slide);
  const edited = `${md}\n<!-- block:handmade -->\n새 블록\n`;
  const back = readSlide({ md: edited, layout });
  assert.equal(back.blocks.length, slide.blocks.length + 1);
  const added = back.blocks.find((b) => b.id === 'handmade');
  assert.ok(added, 'the hand-added block survived');
  assert.ok(added.w > 0, 'and received geometry');
});

test('layout entries for deleted blocks are dropped', () => {
  const slide = makeSlide('title-content', { title: 'A' });
  const { layout } = writeSlide(slide);
  const back = readSlide({ md: '<!-- block:only -->\n남은 블록', layout });
  assert.equal(back.blocks.length, 1);
  assert.equal(back.blocks[0].id, 'only');
});

test('blocks are clamped inside the canvas', () => {
  const slide = readSlide({
    md: '<!-- block:b1 -->\nx',
    layout: { canvas: { w: 1280, h: 720 }, blocks: { b1: { x: 5000, y: -100, w: 400, h: 100 } } },
  });
  const b = slide.blocks[0];
  assert.ok(b.x + b.w <= 1280 && b.y >= 0, `clamped to ${JSON.stringify(b)}`);
});

/* -------------------------------------------------------------------- doc */

test('doc section round-trips', () => {
  const section = makeSection({ name: '서론' });
  const { md, meta } = writeSection(section);
  const back = readSection({ md, meta });
  assert.equal(back.name, '서론');
  assert.deepEqual(back.blocks.map((b) => b.md), section.blocks.map((b) => b.md));
});

test('doc markdown stays marker-free when formatting is default', () => {
  const { md } = writeSection(makeSection({ name: '깔끔' }));
  assert.ok(!md.includes('<!-- block:'), `expected clean markdown, got:\n${md}`);
});

test('a block with an override gains an anchor, and loses it when reset', () => {
  const section = makeSection({ name: '서식' });
  section.blocks[1].override = { align: 'center', style: { italic: true } };

  const withOverride = writeSection(section);
  assert.ok(withOverride.md.includes('<!-- block:'), 'anchor appears for the styled block');
  assert.equal(Object.keys(withOverride.meta.blocks).length, 1);

  const reopened = readSection(withOverride);
  const styled = reopened.blocks.find((b) => b.override);
  assert.equal(styled.override.align, 'center');
  assert.equal(styled.override.style.italic, true);

  styled.override = null;
  const cleaned = writeSection(reopened);
  assert.ok(!cleaned.md.includes('<!-- block:'), 'anchor disappears once formatting is default');
  assert.deepEqual(cleaned.meta.blocks, {});
});

test('an override equal to the default is not persisted', () => {
  const section = makeSection({ name: 'x' });
  section.blocks[0].override = { align: 'left', indent: 0 };
  const { meta } = writeSection(section);
  assert.deepEqual(meta.blocks, {}, 'default-valued overrides carry no information');
});

test('plain markdown pasted into a section is segmented into blocks', () => {
  const section = readSection({ md: '# 제목\n\n문단 1\n\n문단 2', meta: null });
  assert.equal(section.blocks.length, 3);
  assert.deepEqual(section.blocks.map((b) => b.type), ['heading', 'paragraph', 'paragraph']);
});

test('the outline and stats are derived on save', () => {
  const section = readSection({ md: '# 상위\n\n본문 텍스트\n\n## 하위', meta: null });
  const { meta } = writeSection(section);
  assert.deepEqual(meta.outline.map((o) => [o.level, o.text]), [[1, '상위'], [2, '하위']]);
  assert.ok(meta.stats.words > 0);
});

/* ------------------------------------------------------------------ grid */

test('sheet round-trips with formulas intact', () => {
  const sheet = makeSheet({ name: '매출', withSample: true });
  const { md, cells } = writeSheet(sheet);
  const back = readSheet({ md, cells });

  assert.equal(back.name, '매출');
  assert.equal(back.cells.D2.f, '=B2+C2');
  assert.equal(back.cells.A1.v, '항목');
  assert.equal(back.cells.A1.style.bold, true);
});

test('the markdown projection shows computed values, not formulas, in the table', () => {
  const { md, cells } = writeSheet(makeSheet({ name: '매출', withSample: true }));

  assert.equal(cells.cells.D2.v, 2550, 'the json cached the computed value');
  assert.match(md, /2,550/, 'the table shows the formatted result');

  const tablePart = md.slice(0, md.indexOf('### 수식'));
  assert.ok(!tablePart.includes('=B2+C2'), 'formulas do not appear inside the table');
  assert.match(md, /`D2` = `=B2\+C2` → 2,550/, 'but they do appear in the formula appendix');
});

test('the projection includes column letters and row numbers for cell-address reasoning', () => {
  const { md } = writeSheet(makeSheet({ name: 's', withSample: true }));
  assert.match(md, /\|\s+\|\s*A\s*\|\s*B\s*\|/, 'column letters header');
  assert.match(md, /\|\s*\*\*1\*\*\s*\|/, 'row numbers');
});

test('the projection carries a per-column summary', () => {
  const { md } = writeSheet(makeSheet({ name: 's', withSample: true }));
  assert.match(md, /### 열 요약/);
  assert.match(md, /\| B \| 1분기 \| 숫자 \|/);
});

test('a hand-written markdown table is importable', () => {
  const cells = importMarkdownTable('| 이름 | 점수 |\n|---|---|\n| 가 | 90 |\n| 나 | 85 |');
  assert.equal(cells.A1.v, '이름');
  assert.equal(cells.B2.v, 90);
  assert.equal(cells.B2.t, 'n');
  assert.equal(cells.A3.v, '나');
});

test('the projection format imports back to the same cells', () => {
  const sheet = makeSheet({ name: '왕복', withSample: true });
  const { md } = writeSheet(sheet);
  const reimported = importMarkdownTable(md);
  assert.equal(reimported.A1.v, '항목');
  assert.equal(reimported.A1.style.bold, true, 'bold survived as ** markers');
  assert.equal(reimported.B2.v, 1200);
  assert.equal(reimported.D2.v, 2550, 'the computed value came back as a literal');
});

test('opening a sheet with only markdown imports the table', () => {
  const md = '---\nid: sh1\nname: 수동시트\n---\n\n| 항목 | 값 |\n|---|---|\n| 가 | 5 |';
  const sheet = readSheet({ md, cells: null });
  assert.equal(sheet.name, '수동시트');
  assert.equal(sheet.cells.A1.v, '항목');
  assert.equal(sheet.cells.B2.v, 5);
});

test('cells json wins over the markdown projection when both exist', () => {
  const md = '| 항목 |\n|---|\n| 오래된값 |';
  const sheet = readSheet({ md, cells: { name: 's', cells: { A1: { v: '최신값', t: 's' } } } });
  assert.equal(sheet.cells.A1.v, '최신값');
});

test('empty cells are not persisted', () => {
  const { cells } = writeSheet({ name: 's', cells: { A1: { v: 1 }, B1: { v: '' }, C1: {} } });
  assert.ok('A1' in cells.cells);
  assert.ok(!('B1' in cells.cells), 'blank cell dropped');
  assert.ok(!('C1' in cells.cells), 'empty object dropped');
});

test('pipes in cell text do not break the markdown table', () => {
  const md = renderSheetMarkdown({
    name: 's', id: 'x', names: {},
    cells: { A1: { v: 'a|b', t: 's' } },
  });
  const row = md.split('\n').find((l) => l.startsWith('| **1**'));
  assert.match(row, /a\\\|b/, 'the pipe was escaped');
});

/* ---------------------------------------------------------------- digest */

test('deck digest describes positions in words, not pixels', () => {
  const slide = makeSlide('title', { title: '다이제스트 확인' });
  slide.notes = '노트 내용';
  const digest = buildDigest({
    type: 'deck',
    manifest: { title: '테스트 덱', modified: '2026-09-01T00:00:00.000Z' },
    slides: [slide],
  });

  assert.match(digest, /# 테스트 덱/);
  assert.match(digest, /## 슬라이드 1/);
  assert.match(digest, /상단 중앙|정중앙|중앙/, 'positions are prose');
  assert.match(digest, /발표자 노트: 노트 내용/);
  assert.match(digest, /# 다이제스트 확인/, 'the slide text is included verbatim');
  assert.ok(!/x=\d+/.test(digest), 'no raw pixel coordinates');
});

test('grid digest turns rows into labelled records', () => {
  const digest = buildDigest({
    type: 'grid',
    manifest: { title: '테스트 시트', modified: '2026-09-01T00:00:00.000Z' },
    sheets: [writeSheetForDigest()],
  });
  assert.match(digest, /### 열 구성/);
  assert.match(digest, /2행: 항목=제품 A, 1분기=1,200/, 'rows read as key=value records');
  assert.match(digest, /### 수식과 계산 결과/);
  assert.match(digest, /`D2`: `=B2\+C2` → \*\*2,550\*\*/);
});

function writeSheetForDigest() {
  const sheet = makeSheet({ name: '매출', withSample: true });
  const { cells } = writeSheet(sheet);
  return { ...sheet, cells: cells.cells };
}

test('doc digest carries an outline and the full text', () => {
  const section = readSection({ md: '# 제목\n\n본문입니다.\n\n## 소제목', meta: null });
  const digest = buildDigest({
    type: 'doc',
    manifest: { title: '테스트 문서', modified: '2026-09-01T00:00:00.000Z' },
    sections: [section],
  });
  assert.match(digest, /## 목차/);
  assert.match(digest, /- 제목/);
  assert.match(digest, /본문입니다\./);
  assert.match(digest, /단어: 약 \d+개/);
});

/* ------------------------------------------- column summary correctness */

test('the column summary stops at the first blank row', async () => {
  const { tableExtent, summarizeColumns } = await import('../src/grid.js');
  const sheet = {
    name: 's', id: 'x', names: {},
    cells: {
      A1: { v: '항목', t: 's' }, B1: { v: '값', t: 's' },
      A2: { v: '가', t: 's' }, B2: { v: 10, t: 'n' },
      A3: { v: '나', t: 's' }, B3: { v: 20, t: 'n' },
      // blank row 4, then an unrelated block that must not join the sum
      A5: { v: '메모', t: 's' }, B5: { v: 9999, t: 'n' },
    },
  };
  assert.equal(tableExtent(sheet.cells, 4, 1), 2, 'table ends at row index 2 (row 3)');
  const b = summarizeColumns(sheet).find((c) => c.col === 'B');
  assert.equal(b.sum, '30', `expected 30, got ${b.sum}`);
  assert.equal(b.count, 2);
});

test('the column summary excludes a labelled total row', async () => {
  const { summarizeColumns, summaryScope } = await import('../src/grid.js');
  const sheet = {
    name: 's', id: 'x', names: {},
    cells: {
      A1: { v: '항목', t: 's' }, B1: { v: '값', t: 's' },
      A2: { v: '가', t: 's' }, B2: { v: 10, t: 'n' },
      A3: { v: '나', t: 's' }, B3: { v: 20, t: 'n' },
      A4: { v: '합계', t: 's' }, B4: { f: '=SUM(B2:B3)', v: 30, t: 'n' },
    },
  };
  const b = summarizeColumns(sheet).find((c) => c.col === 'B');
  assert.equal(b.sum, '30', 'the total row is not counted twice');
  assert.deepEqual(summaryScope(sheet).excluded, [4]);
});

test('an unlabelled total row is still detected by its formula shape', async () => {
  const { summarizeColumns } = await import('../src/grid.js');
  const sheet = {
    name: 's', id: 'x', names: {},
    cells: {
      B1: { v: 'Value', t: 's' },
      B2: { v: 5, t: 'n' },
      B3: { v: 15, t: 'n' },
      B4: { f: '=SUM(B2:B3)', v: 20, t: 'n' },
    },
  };
  const b = summarizeColumns(sheet).find((c) => c.col === 'B');
  assert.equal(b.sum, '20', `expected 20 (5+15), got ${b.sum}`);
  assert.equal(b.max, '15', 'the total is excluded from max too');
});

test('the summary reconciles with the sheet total for a budget-shaped sheet', async () => {
  const { makeSheet, writeSheet, summarizeColumns } = await import('../src/grid.js');
  const sheet = makeSheet({ name: '예산', withSample: true });
  const { cells } = writeSheet(sheet);
  const withValues = { ...sheet, cells: cells.cells };

  const b = summarizeColumns(withValues).find((c) => c.col === 'B');
  // The sample's own total row is B5 = SUM(B2:B4) = 1200+980+640 = 2820.
  assert.equal(b.sum, '2,820', `column sum ${b.sum} must equal the sheet's own total`);
});

test('percentage columns are summarised as percentages', async () => {
  const { summarizeColumns } = await import('../src/grid.js');
  const sheet = {
    name: 's', id: 'x', names: {},
    cells: {
      A1: { v: '항목', t: 's' }, B1: { v: '비중', t: 's' },
      A2: { v: '가', t: 's' }, B2: { v: 0.25, t: 'n', fmt: '0.0%' },
      A3: { v: '나', t: 's' }, B3: { v: 0.75, t: 'n', fmt: '0.0%' },
    },
  };
  const b = summarizeColumns(sheet).find((c) => c.col === 'B');
  assert.equal(b.sum, '100%', `expected 100%, got ${b.sum}`);
});
