/**
 * Render the real React app in jsdom and assert it mounts without errors.
 *
 * This is not a substitute for looking at it in a browser, but it does execute
 * every editor's render path against real project data, which catches the
 * failure mode a build cannot: a runtime error inside a component.
 *
 * Needs `jsdom` and `esbuild`, which live in the scratchpad rather than in the
 * project's dependencies:
 *   node scripts/ui-smoke.mjs
 */
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..');
const TOOLS = process.env.AI_STUDIO_TOOLS ?? process.env.SCRATCHPAD;

if (!TOOLS) {
  console.error('AI_STUDIO_TOOLS 환경변수에 jsdom/esbuild가 설치된 폴더를 지정하세요.');
  process.exit(2);
}

const require = createRequire(pathToFileURL(path.join(TOOLS, 'package.json')));
const { JSDOM } = require('jsdom');
const esbuild = require('esbuild');

let failures = 0;
let checks = 0;
const check = (label, ok, detail) => {
  checks++;
  if (ok) console.log(`  ✓ ${label}`);
  else {
    failures++;
    console.log(`  ✗ ${label}${detail ? `\n      ${detail}` : ''}`);
  }
};

/* ------------------------------------------------------------ build bundle */

const bundle = await esbuild.build({
  entryPoints: [path.join(ROOT, 'apps/web/src/main.jsx')],
  bundle: true,
  write: false,
  format: 'iife',
  platform: 'browser',
  jsx: 'automatic',
  target: 'es2022',
  define: { 'process.env.NODE_ENV': '"development"' },
  loader: { '.css': 'text' },
  alias: {
    '@ai-studio/format/browser': path.join(ROOT, 'packages/format/src/browser.js'),
    '@ai-studio/formula': path.join(ROOT, 'packages/formula/src/index.js'),
  },
  absWorkingDir: TOOLS,
  nodePaths: [path.join(ROOT, 'node_modules')],
});
const code = bundle.outputFiles[0].text;
console.log(`\n번들 생성: ${(code.length / 1024).toFixed(0)}KB`);

/* --------------------------------------------------------------- fixtures */

const { createProject } = await import(pathToFileURL(path.join(ROOT, 'packages/format/src/project.js')));
const { promises: fs } = await import('node:fs');
const os = await import('node:os');

const tmp = await fs.mkdtemp(path.join(os.tmpdir(), 'ai-studio-smoke-'));
const fixtures = {};
for (const type of ['deck', 'doc', 'grid']) {
  const project = await createProject(tmp, { type, title: `스모크 ${type}` });
  // The slideshow's notes pane can only be checked if a slide has notes.
  if (type === 'deck') project.slides.forEach((s, i) => { s.notes = `슬라이드 ${i + 1} 발표자 노트`; });
  fixtures[path.basename(project.dir)] = {
    type: project.type,
    folder: path.basename(project.dir),
    manifest: project.manifest,
    ...(type === 'deck' ? { slides: project.slides } : {}),
    ...(type === 'doc' ? { sections: project.sections } : {}),
    ...(type === 'grid' ? { sheets: project.sheets } : {}),
  };
}
const folders = Object.keys(fixtures);
console.log(`픽스처: ${folders.join(', ')}\n`);

/* ------------------------------------------------------------ jsdom harness */

async function mount(hash, { interact } = {}) {
  const dom = new JSDOM('<!doctype html><html><body><div id="root"></div></body></html>', {
    url: `http://localhost/${hash}`,
    pretendToBeVisual: true,
    runScripts: 'outside-only',
  });
  const { window } = dom;
  const errors = [];

  window.addEventListener('error', (e) => errors.push(`window.error: ${e.message}`));

  // jsdom lacks these; the editors use them for layout and clipboard.
  window.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
  Object.defineProperty(window.navigator, 'clipboard', {
    value: { writeText: async () => {} },
    configurable: true,
  });
  window.requestAnimationFrame = (cb) => setTimeout(() => cb(Date.now()), 0);
  window.cancelAnimationFrame = (id) => clearTimeout(id);
  window.scrollTo = () => {};
  window.Element.prototype.scrollIntoView = () => {};

  // jsdom has no Response; Node's (undici) works with the app's api.js as-is.
  const NodeResponse = globalThis.Response;

  window.fetch = async (url, options = {}) => {
    const u = String(url);
    const json = (data, status = 200) =>
      new NodeResponse(JSON.stringify(data), {
        status,
        headers: { 'Content-Type': 'application/json' },
      });

    if (u.endsWith('/api/projects') && (!options.method || options.method === 'GET')) {
      return json({
        workspace: tmp,
        projects: folders.map((folder) => ({
          type: fixtures[folder].type,
          folder,
          title: fixtures[folder].manifest.title,
          label: '문서',
          count: 1,
          modified: new Date().toISOString(),
        })),
      });
    }
    const match = u.match(/\/api\/projects\/([^/?]+)$/);
    if (match) {
      const folder = decodeURIComponent(match[1]);
      if (!fixtures[folder]) return json({ error: 'not found' }, 404);
      return json(fixtures[folder]);
    }
    return json({ error: `unhandled ${u}` }, 500);
  };

  // React logs errors through console.error; treat those as failures too.
  const realError = console.error;
  console.error = (...args) => {
    const text = args.map(String).join(' ');
    if (!/not wrapped in act|useLayoutEffect does nothing on the server/.test(text)) {
      errors.push(`console.error: ${text.slice(0, 400)}`);
    }
  };
  window.console = { ...console, error: console.error };

  // React 18 + jsdom needs these globals present before the bundle evaluates.
  const globalsBefore = {
    window: global.window,
    document: global.document,
    navigator: global.navigator,
    fetch: global.fetch,
  };
  global.window = window;
  global.document = window.document;
  global.navigator = window.navigator;
  // The bundle calls bare `fetch`, which resolves against globalThis rather than
  // the `window` we pass in, so the stub has to be installed globally too.
  global.fetch = window.fetch;
  global.HTMLElement = window.HTMLElement;
  global.Element = window.Element;
  global.Node = window.Node;
  // Bare identifiers in the bundle resolve against globalThis, so the browser
  // APIs jsdom lacks have to be visible there too, not only on `window`.
  globalsBefore.ResizeObserver = global.ResizeObserver;
  globalsBefore.matchMedia = global.matchMedia;
  global.ResizeObserver = window.ResizeObserver;
  global.matchMedia = window.matchMedia;
  global.getComputedStyle = window.getComputedStyle.bind(window);
  global.requestAnimationFrame = window.requestAnimationFrame;
  global.cancelAnimationFrame = window.cancelAnimationFrame;
  global.IS_REACT_ACT_ENVIRONMENT = false;

  try {
    // eslint-disable-next-line no-new-func
    const run = new Function('window', 'document', 'navigator', 'self', 'globalThis', code);
    run(window, window.document, window.navigator, window, global);
  } catch (e) {
    errors.push(`bundle threw: ${e.message}`);
  }

  const settle = async (rounds = 30) => {
    for (let i = 0; i < rounds; i++) await new Promise((r) => setTimeout(r, 12));
  };
  await settle();

  // Interactions must run before the globals are restored, since React's event
  // handlers reach for document/window through globalThis.
  if (interact) {
    try {
      await interact({ window, settle, ...makeDriver(window) });
    } catch (e) {
      errors.push(`interact threw: ${e.message}`);
    }
    await settle(10);
  }

  const html = window.document.getElementById('root')?.innerHTML ?? '';
  const text = window.document.body.textContent ?? '';

  console.error = realError;
  for (const [key, value] of Object.entries(globalsBefore)) {
    if (value === undefined) delete global[key];
    else global[key] = value;
  }

  return { window, html, text, errors, dom };
}

/* ------------------------------------------------------------------ driver */

/**
 * Minimal interaction driver.
 *
 * React listens on the root container, so dispatching real DOM events works; the
 * one catch is controlled inputs, which need the value set through the native
 * property setter or React never sees the change.
 */
function makeDriver(window) {
  const { document } = window;

  const fire = (el, type, init = {}) => {
    const Ctor =
      type.startsWith('key') ? window.KeyboardEvent
      : type.startsWith('mouse') || type === 'click' || type === 'dblclick' || type === 'contextmenu'
        ? window.MouseEvent
      : type.startsWith('pointer') ? (window.PointerEvent ?? window.MouseEvent)
      : window.Event;
    el.dispatchEvent(new Ctor(type, { bubbles: true, cancelable: true, ...init }));
  };

  const setValue = (el, value) => {
    const proto = el.tagName === 'TEXTAREA' ? window.HTMLTextAreaElement : window.HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(proto.prototype, 'value').set;
    setter.call(el, value);
    fire(el, 'input');
  };

  const fireWindow = (type, init = {}) => {
    window.dispatchEvent(new window.Event(type, { bubbles: true, ...init }));
  };

  return {
    $: (sel) => document.querySelector(sel),
    $$: (sel) => [...document.querySelectorAll(sel)],
    fire,
    fireWindow,
    setValue,
    /** Key event on window, for the app-level shortcut handlers. */
    winKey: (key, init = {}) =>
      window.dispatchEvent(new window.KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init })),
    click: (el) => {
      fire(el, 'mousedown', { button: 0 });
      fire(el, 'mouseup', { button: 0 });
      fire(el, 'click', { button: 0 });
    },
    dblclick: (el) => {
      fire(el, 'mousedown', { button: 0 });
      fire(el, 'dblclick', { button: 0 });
    },
    key: (el, key, init = {}) => fire(el, 'keydown', { key, ...init }),
    /** Text of the cell at a 1-based row / 0-based column in the grid. */
    cellText: (row, col) => {
      const tr = document.querySelectorAll('.sheet tbody tr')[row - 1];
      return tr?.querySelectorAll('td')[col]?.textContent ?? '';
    },
  };
}

/* ------------------------------------------------------------------- checks */

console.log('■ 시작 화면 (Launcher)');
{
  const { html, text, errors } = await mount('');
  check('마운트에 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('앱 제목 렌더링', text.includes('AI Studio'));
  check('세 앱 카드 표시', text.includes('AI Deck') && text.includes('AI Doc') && text.includes('AI Grid'));
  check('최근 문서 목록 렌더링', folders.every((f) => html.includes(f)), html.slice(0, 300));
}

const byType = Object.fromEntries(folders.map((f) => [fixtures[f].type, f]));

console.log('\n■ Deck 에디터');
{
  const { text, html, errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`);
  check('마운트에 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('리본 탭 렌더링', ['홈', '삽입', '디자인', 'AI'].every((t) => text.includes(t)));
  check('슬라이드 패널 렌더링', text.includes('슬라이드'));
  check('캔버스 존재', html.includes('class="canvas"'), html.slice(0, 200));
  check('슬라이드 본문이 렌더링됨', text.includes('스모크 deck'));
  check('발표자 노트 영역 존재', text.includes('발표자 노트'));
  check('저장 포맷 패널이 md를 보여줌', text.includes('.md') && text.includes('layout.json'), sample(text));
  check('AI.md 탭 존재', text.includes('AI.md'));
}

console.log('\n■ Doc 에디터');
{
  const { text, html, errors } = await mount(`#/doc/${encodeURIComponent(byType.doc)}`);
  check('마운트에 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('리본 탭 렌더링', ['홈', '삽입', '레이아웃'].every((t) => text.includes(t)));
  check('페이지 렌더링', html.includes('class="page"'));
  check('문서 본문이 렌더링됨', text.includes('내용을 입력하세요') || text.includes('스모크 doc'));
  check('개요 패널 존재', text.includes('개요'));
  check('단어 수 상태바 존재', /단어 \d+/.test(text), sample(text));
  check('meta.json 탭 존재', text.includes('meta.json'));
}

console.log('\n■ Grid 에디터');
{
  const { text, html, errors } = await mount(`#/grid/${encodeURIComponent(byType.grid)}`);
  check('마운트에 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('리본 탭 렌더링', ['홈', '삽입', '데이터'].every((t) => text.includes(t)));
  check('수식 입력줄 존재', html.includes('formulabar'));
  check('열 머리글 A/B/C 렌더링', html.includes('>A<') && html.includes('>B<'));
  check('샘플 데이터 렌더링', text.includes('제품 A'), sample(text));
  check('수식 결과가 계산되어 표시됨', text.includes('2,550'), sample(text));
  check('총계 행 렌더링', text.includes('총계'));
  check('시트 탭 존재', html.includes('sheettab'));
  check('cells.json 탭 존재', text.includes('cells.json'));
}

/* -------------------------------------------------------- interactions */

console.log('\n■ Grid 편집 상호작용');
{
  const captured = { md: null, json: null };
  const { text, errors } = await mount(`#/grid/${encodeURIComponent(byType.grid)}`, {
    async interact({ window, settle, $, $$, click, key, setValue, cellText }) {
      // Select B2 (the first quarter figure, 1200) and retype it.
      const rows = $$('.sheet tbody tr');
      const b2 = rows[1].querySelectorAll('td')[1];
      click(b2);
      await settle(4);

      key(window.document.body, '7');
      await settle(4);

      const editor = $('.cell__editor');
      if (!editor) throw new Error('typing a character did not open the cell editor');
      setValue(editor, '5000');
      key(editor, 'Enter');
      await settle(8);

      captured.b2 = cellText(2, 1);
      captured.d2 = cellText(2, 3);
      captured.total = cellText(5, 1);

      // The formula bar should reflect the newly selected cell (B3 after Enter).
      captured.ref = $('.formulabar__ref')?.value;

      // The inspector's json tab should show the edited value.
      const jsonTab = $$('.inspector__tab').find((t) => t.textContent.includes('cells.json'));
      if (jsonTab) {
        click(jsonTab);
        await settle(4);
        captured.json = $('.code')?.textContent ?? '';
      }
    },
  });

  check('편집 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('입력한 값이 셀에 반영됨', captured.b2?.includes('5,000'), `B2 = ${captured.b2}`);
  check('의존 수식이 재계산됨 (D2 = B2+C2)', captured.d2?.includes('6,350'),
    `D2 = ${captured.d2} (5000 + 1350 이어야 함)`);
  check('합계 행도 재계산됨 (B5 = SUM(B2:B4))', captured.total?.includes('6,620'),
    `B5 = ${captured.total} (5000+980+640 = 6620 이어야 함)`);
  check('Enter가 아래 셀로 이동', captured.ref === 'B3', `ref = ${captured.ref}`);
  check('저장 포맷 패널의 json에 새 값이 보임', captured.json?.includes('5000'),
    (captured.json ?? '').slice(0, 200));
}

console.log('\n■ Deck 편집 상호작용');
{
  const captured = {};
  const { errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    async interact({ settle, $, $$, dblclick, setValue }) {
      const block = $('.canvas .block');
      if (!block) throw new Error('no block on the canvas');
      dblclick(block);
      await settle(4);

      const editor = $('.block__editor');
      if (!editor) throw new Error('double-click did not open the block editor');
      setValue(editor, '# 편집된 제목\n\n두 번째 줄');
      await settle(6);

      const mdTab = $$('.inspector__tab').find((t) => t.textContent.endsWith('.md'));
      if (mdTab) {
        mdTab.dispatchEvent(new (globalThis.window.MouseEvent)('click', { bubbles: true }));
        await settle(4);
        captured.md = $('.code')?.textContent ?? '';
      }
      captured.thumb = $('.sorter-item__thumb')?.textContent ?? '';
    },
  });

  check('편집 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('입력한 마크다운이 md 파일 내용에 반영됨', captured.md?.includes('# 편집된 제목'),
    (captured.md ?? '').slice(0, 240));
  check('md에 좌표가 섞이지 않음', captured.md ? !/"x":/.test(captured.md) : false);
  check('썸네일이 갱신됨', captured.thumb?.includes('편집된 제목'), `thumb = ${captured.thumb}`);
}

console.log('\n■ Doc 편집 상호작용');
{
  const captured = {};
  const { errors } = await mount(`#/doc/${encodeURIComponent(byType.doc)}`, {
    async interact({ settle, $, $$, click, setValue, key }) {
      const blocks = $$('.docblock');
      if (!blocks.length) throw new Error('no blocks in the document');
      click(blocks[0]);
      await settle(4);

      const editor = $('.docblock__editor');
      if (!editor) throw new Error('clicking a paragraph did not open the editor');
      setValue(editor, '## 새 소제목');
      await settle(6);
      captured.outline = $$('.outline-item').map((b) => b.textContent).join(' | ');

      const mdTab = $$('.inspector__tab').find((t) => t.textContent.endsWith('.md'));
      if (mdTab) {
        click(mdTab);
        await settle(4);
        captured.md = $('.code')?.textContent ?? '';
      }
    },
  });

  check('편집 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('입력한 내용이 md에 반영됨', captured.md?.includes('## 새 소제목'),
    (captured.md ?? '').slice(0, 200));
  check('기본 서식이므로 md에 앵커가 없음', captured.md ? !captured.md.includes('<!-- block:') : false,
    (captured.md ?? '').slice(0, 200));
  check('개요 패널이 새 제목을 반영', captured.outline?.includes('새 소제목'),
    `outline = ${captured.outline}`);
}

/* ------------------------------------------------- new interaction surfaces */

console.log('\n■ Grid 자동 채우기 · 크기 조절 · 이름 상자');
{
  const got = {};
  const { errors } = await mount(`#/grid/${encodeURIComponent(byType.grid)}`, {
    async interact({ window, settle, $, $$, click, fire, setValue, cellText }) {
      // Type a series into B8/B9, then drag the fill handle down to B11.
      const rows = () => $$('.sheet tbody tr');
      const cellAt = (r, c) => rows()[r - 1].querySelectorAll('td')[c];

      for (const [row, value] of [[8, '10'], [9, '20']]) {
        click(cellAt(row, 1));
        await settle(3);
        window.dispatchEvent(new window.KeyboardEvent('keydown', { key: '1', bubbles: true, cancelable: true }));
        await settle(3);
        const editor = $('.cell__editor');
        setValue(editor, value);
        fire(editor, 'keydown', { key: 'Enter' });
        await settle(5);
      }

      // Select B8:B9 by shift-clicking, then drag the handle.
      click(cellAt(8, 1));
      await settle(3);
      fire(cellAt(9, 1), 'mousedown', { button: 0, shiftKey: true });
      fire(cellAt(9, 1), 'mouseup', { button: 0 });
      await settle(4);

      const handle = $('.fillhandle');
      got.hasHandle = !!handle;
      if (handle) {
        fire(handle, 'mousedown', { button: 0 });
        await settle(2);
        fire(cellAt(11, 1), 'mouseover');
        await settle(2);
        got.previewCount = $$('td.is-fillpreview').length;
        window.dispatchEvent(new window.Event('mouseup', { bubbles: true }));
        await settle(6);
      }
      got.b10 = cellText(10, 1);
      got.b11 = cellText(11, 1);

      // Name box navigation.
      const nameBox = $('.formulabar__ref');
      setValue(nameBox, 'D3');
      fire(nameBox, 'keydown', { key: 'Enter' });
      await settle(4);
      got.nameBoxRef = $('.formulabar__ref')?.value;

      // Column resize via the header grabber.
      const resizer = $('.resizer--col');
      got.hasResizer = !!resizer;
      if (resizer) {
        fire(resizer, 'mousedown', { button: 0, clientX: 100 });
        // The window listeners attach in an effect, so let React commit first —
        // in a browser there are frames between pressing and moving.
        await settle(4);
        window.dispatchEvent(new window.MouseEvent('mousemove', { clientX: 180, bubbles: true }));
        await settle(3);
        window.dispatchEvent(new window.MouseEvent('mouseup', { bubbles: true }));
        await settle(6);
      }
      got.colWidth = $$('.sheet colgroup col')[1]?.style?.width;
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('선택 영역에 채우기 핸들이 표시됨', got.hasHandle === true);
  check('끄는 동안 미리보기가 표시됨', (got.previewCount ?? 0) >= 2, `preview cells: ${got.previewCount}`);
  check('자동 채우기가 등차수열을 이어감', got.b10?.includes('30'), `B10 = ${got.b10}`);
  check('자동 채우기가 끝까지 채움', got.b11?.includes('40'), `B11 = ${got.b11}`);
  check('이름 상자로 이동', got.nameBoxRef === 'D3', `ref = ${got.nameBoxRef}`);
  check('열 경계에 크기 조절 손잡이가 있음', got.hasResizer === true);
  check('열 너비가 실제로 변경됨', got.colWidth && parseInt(got.colWidth, 10) > 110, `width = ${got.colWidth}`);
}

console.log('\n■ Grid 병합 · 컨텍스트 메뉴');
{
  const got = {};
  const { errors } = await mount(`#/grid/${encodeURIComponent(byType.grid)}`, {
    async interact({ settle, $, $$, click, fire }) {
      const cellAt = (r, c) => $$('.sheet tbody tr')[r - 1].querySelectorAll('td')[c];

      click(cellAt(1, 1));
      await settle(3);
      fire(cellAt(1, 2), 'mousedown', { button: 0, shiftKey: true });
      fire(cellAt(1, 2), 'mouseup', { button: 0 });
      await settle(3);

      // Right-click opens the cell menu.
      fire(cellAt(1, 1), 'contextmenu', { clientX: 120, clientY: 200 });
      await settle(4);
      got.menuItems = $$('.ctxmenu__item').map((b) => b.textContent);

      const mergeItem = $$('.ctxmenu__item').find((b) => b.textContent.includes('셀 병합'));
      if (mergeItem) {
        click(mergeItem);
        await settle(6);
      }
      got.colSpan = $$('.sheet tbody td').find((td) => td.getAttribute('colspan'))?.getAttribute('colspan');
      got.menuClosed = $$('.ctxmenu').length === 0;
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('우클릭 메뉴가 열림', (got.menuItems ?? []).some((t) => t.includes('복사')), JSON.stringify(got.menuItems));
  check('메뉴에 셀 병합이 있음', (got.menuItems ?? []).some((t) => t.includes('셀 병합')));
  check('병합이 colspan으로 반영됨', got.colSpan === '2', `colspan = ${got.colSpan}`);
  check('선택 후 메뉴가 닫힘', got.menuClosed === true);
}

console.log('\n■ Deck 슬라이드 쇼 · 정렬 · 복제');
{
  const got = {};
  const { errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    async interact({ window, settle, $, $$, click, fire, winKey }) {
      const block = $('.canvas .block');
      fire(block, 'pointerdown', { button: 0, pointerId: 1 });
      fire(block, 'pointerup', { button: 0, pointerId: 1 });
      await settle(4);
      got.selected = !!$('.block.is-selected');

      const before = $$('.canvas .block').length;
      winKey('d', { ctrlKey: true });
      await settle(5);
      got.duplicated = $$('.canvas .block').length === before + 1;

      // Align the selected block to the left edge of the canvas.
      const alignBtn = $$('.rbtn').find((b) => b.title === '개체 왼쪽 맞춤');
      got.hasAlign = !!alignBtn;
      if (alignBtn) {
        click(alignBtn);
        await settle(5);
        got.leftMost = $('.block.is-selected')?.style?.left;
      }

      // F5 starts the slideshow.
      winKey('F5');
      await settle(6);
      got.showing = !!$('.slideshow');
      got.showText = $('.slideshow')?.textContent ?? '';

      winKey('ArrowRight');
      await settle(4);
      got.counter = $('.slideshow__count')?.textContent?.trim();

      winKey('s');
      await settle(3);
      got.notesShown = !!$('.slideshow__notes');

      winKey('Escape');
      await settle(4);
      got.closed = !$('.slideshow');
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('블록 선택이 표시됨', got.selected === true);
  check('Ctrl+D가 블록을 복제', got.duplicated === true);
  check('정렬 버튼이 리본에 있음', got.hasAlign === true);
  check('왼쪽 맞춤이 x를 0으로', got.leftMost === '0px', `left = ${got.leftMost}`);
  check('F5가 슬라이드 쇼를 시작', got.showing === true);
  check('슬라이드 쇼가 내용을 렌더링', got.showText.includes('스모크 deck'), got.showText.slice(0, 120));
  check('방향키로 슬라이드 이동', got.counter === '2 / 2', `counter = ${got.counter}`);
  check('S가 발표자 노트를 표시', got.notesShown === true);
  check('Esc로 슬라이드 쇼 종료', got.closed === true);
}

console.log('\n■ Deck 차트 삽입');
{
  const got = {};
  const { errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    async interact({ settle, $, $$, click }) {
      const insertTab = $$('.ribbon__tab').find((t) => t.textContent === '삽입');
      click(insertTab);
      await settle(4);

      const chartBtn = $$('.rbtn').find((b) => b.textContent.includes('차트'));
      got.hasChartBtn = !!chartBtn;
      click(chartBtn);
      await settle(6);

      got.dialogOpen = !!$('.chartdlg');
      got.previewSvg = !!$('.chartdlg__preview svg');

      const confirm = $$('.dialog__foot button').find((b) => b.textContent.includes('삽입'));
      if (confirm) {
        click(confirm);
        await settle(8);
      }
      got.chartOnCanvas = !!$('.canvas .chart-view svg');
      const mdTab = $$('.inspector__tab').find((t) => t.textContent.endsWith('.md'));
      if (mdTab) {
        click(mdTab);
        await settle(4);
        got.md = $('.code')?.textContent ?? '';
      }
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('삽입 탭에 차트 버튼이 있음', got.hasChartBtn === true);
  check('차트 대화상자가 열림', got.dialogOpen === true);
  check('대화상자가 미리보기를 그림', got.previewSvg === true);
  check('삽입 후 캔버스에 차트가 그려짐', got.chartOnCanvas === true);
  check('차트가 ```chart 블록으로 저장됨', got.md?.includes('```chart'), (got.md ?? '').slice(0, 200));
}

console.log('\n■ Doc 찾기 · 페이지 나누기');
{
  const got = {};
  const { errors } = await mount(`#/doc/${encodeURIComponent(byType.doc)}`, {
    async interact({ settle, $, $$, click, fire, setValue, winKey }) {
      got.pagesBefore = $$('.page').length;

      // Ctrl+F opens the find bar.
      winKey('f', { ctrlKey: true });
      await settle(5);
      got.findOpen = !!$('.findbar');

      const input = $('.findbar input[type="text"]');
      setValue(input, '내용');
      await settle(5);
      got.count = $('.findbar__count')?.textContent?.trim();
      got.marks = $$('mark.findhit').length;

      fire(input, 'keydown', { key: 'Escape' });
      await settle(3);
      got.findClosed = !$('.findbar');

      // Insert a page break through the 삽입 tab.
      const insertTab = $$('.ribbon__tab').find((t) => t.textContent === '삽입');
      click(insertTab);
      await settle(4);
      const breakBtn = $$('.rbtn').find((b) => b.textContent.includes('페이지 나누기'));
      got.hasBreakBtn = !!breakBtn;
      if (breakBtn) {
        click(breakBtn);
        await settle(8);
      }
      got.pagesAfter = $$('.page').length;
      got.pageNums = $$('.pagenum').map((n) => n.textContent.trim());
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('문서가 페이지로 렌더링됨', (got.pagesBefore ?? 0) >= 1, `pages = ${got.pagesBefore}`);
  check('Ctrl+F가 찾기 바를 열음', got.findOpen === true);
  check('찾은 결과 수를 표시', /\d+ \/ \d+|결과 없음/.test(got.count ?? ''), `count = ${got.count}`);
  check('본문에 하이라이트 표시', (got.marks ?? 0) > 0, `marks = ${got.marks}`);
  check('Esc로 찾기 닫힘', got.findClosed === true);
  check('페이지 나누기 버튼이 있음', got.hasBreakBtn === true);
  check('페이지 나누기가 페이지를 추가', got.pagesAfter > got.pagesBefore, `${got.pagesBefore} → ${got.pagesAfter}`);
  check('페이지 번호가 표시됨', (got.pageNums ?? []).some((t) => /1 \/ \d+/.test(t)), JSON.stringify(got.pageNums));
}

console.log('\n■ 파일 탭 (세 앱 공통)');
{
  for (const type of ['deck', 'doc', 'grid']) {
    const got = {};
    const { errors } = await mount(`#/${type}/${encodeURIComponent(byType[type])}`, {
      async interact({ settle, $, $$, click }) {
        const fileTab = $$('.ribbon__tab').find((t) => t.textContent === '파일');
        got.hasTab = !!fileTab;
        if (fileTab) {
          click(fileTab);
          await settle(5);
        }
        got.labels = $$('.rbtn').map((b) => b.textContent);
      },
    });
    const labels = (got.labels ?? []).join(' ');
    check(`${type}: 파일 탭 존재`, got.hasTab === true);
    check(`${type}: 새로 만들기 · 열기 · 저장 · 사본`,
      ['새로 만들기', '열기', '저장', '사본'].every((l) => labels.includes(l)), labels.slice(0, 160));
    check(`${type}: 내보내기 항목 존재`, /PowerPoint|Word|Excel/.test(labels), labels.slice(0, 200));
    check(`${type}: AI 다이제스트 내보내기 존재`, labels.includes('AI 다이제스트'));
    check(`${type}: 런타임 오류 없음`, errors.length === 0, errors.join('\n      '));
  }
}

await fs.rm(tmp, { recursive: true, force: true });

console.log(`\n${failures === 0 ? '통과' : '실패'}: ${checks - failures}/${checks} 검사 성공`);
process.exit(failures === 0 ? 0 : 1);

function sample(text) {
  return text.replace(/\s+/g, ' ').slice(0, 300);
}
