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
import fs from 'node:fs';
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

/**
 * The harness uses its own entry point rather than `main.jsx`.
 *
 * In a browser the wasm core arrives over the network; here it is read off disk
 * and handed to `initCore` directly, which keeps the test seam out of the app.
 */
const wasmPath = path.join(ROOT, 'apps/web/src/core/pkg/ai_studio_wasm_bg.wasm');
let wasmBase64;
try {
  const { readFile } = await import('node:fs/promises');
  wasmBase64 = (await readFile(wasmPath)).toString('base64');
} catch {
  console.error(`wasm 코어가 없습니다: ${wasmPath}\nnpm run build:wasm 을 먼저 실행하세요.`);
  process.exit(2);
}

const bundle = await esbuild.build({
  stdin: {
    contents: `
      import React from 'react';
      import { createRoot } from 'react-dom/client';
      import App from './App.jsx';
      import { initCore } from './core/index.js';
      import './styles.css';

      const bytes = Uint8Array.from(atob(AI_STUDIO_WASM_BASE64), (c) => c.charCodeAt(0));
      window.__aiStudioReady = initCore(bytes).then(() => {
        createRoot(document.getElementById('root')).render(React.createElement(App));
      });
    `,
    resolveDir: path.join(ROOT, 'apps/web/src'),
    sourcefile: 'smoke-entry.jsx',
    loader: 'jsx',
  },
  bundle: true,
  write: false,
  format: 'iife',
  platform: 'browser',
  jsx: 'automatic',
  target: 'es2022',
  define: {
    'process.env.NODE_ENV': '"development"',
    AI_STUDIO_WASM_BASE64: JSON.stringify(wasmBase64),
  },
  loader: { '.css': 'text', '.wasm': 'binary' },
  // Only reachable inside the Tauri shell, and this harness is not it.
  external: ['@tauri-apps/api/core'],
  logOverride: {
    // `initCore` is given explicit bytes here, so the module's own URL fallback
    // is dead code; and the stylesheet is loaded as text, not for its effects.
    'empty-import-meta': 'silent',
    'ignored-bare-import': 'silent',
  },
  absWorkingDir: TOOLS,
  nodePaths: [path.join(ROOT, 'node_modules')],
});
const code = bundle.outputFiles[0].text;
console.log(`\n번들 생성: ${(code.length / 1024).toFixed(0)}KB (wasm 코어 포함)`);

/* --------------------------------------------------------------- fixtures */

// Built with the same Rust core the editors use, loaded here as a Node module.
const { loadCore } = await import(pathToFileURL(path.join(ROOT, 'apps/web/test/core.mjs')));
await loadCore();
const core = await import(pathToFileURL(path.join(ROOT, 'apps/web/src/core/index.js')));

const theme = { name: 'aurora', accent: '#4f46e5', font: 'Pretendard' };
const manifestOf = (title, type) => ({
  format: `ai-studio/${type}`,
  formatVersion: 1,
  id: 'prj_smoke1',
  title,
  created: '2026-09-01T00:00:00.000Z',
  modified: '2026-09-01T00:00:00.000Z',
  theme,
});

const fixtures = {};
{
  const slides = [
    core.makeSlide('title', { title: '스모크 deck' }),
    core.makeSlide('title-content', { title: '개요', index: 2 }),
  ];
  // The slideshow's notes pane can only be checked if a slide has notes.
  slides.forEach((s, i) => {
    s.notes = `슬라이드 ${i + 1} 발표자 노트`;
  });
  fixtures['스모크-덱.aideck'] = {
    type: 'deck',
    folder: '스모크-덱.aideck',
    manifest: manifestOf('스모크 deck', 'deck'),
    slides,
  };

  const docSection = core.makeSection({ name: '스모크 doc' });
  const blank = core.blankTable(3, 3);
  docSection.blocks.push({
    id: 'b_doctable',
    md: blank.md,
    type: 'table',
    override: null,
    table: blank.table,
  });
  // A paragraph with its own size, so the editor's style inheritance can be
  // checked: the edit box must look like the rendered paragraph, not shrink.
  docSection.blocks.push({
    id: 'b_docstyled',
    md: '큰 글씨 문단',
    type: 'text',
    override: { style: { fontSize: 22 } },
  });
  fixtures['스모크-문서.aidoc'] = {
    type: 'doc',
    folder: '스모크-문서.aidoc',
    manifest: manifestOf('스모크 doc', 'doc'),
    sections: [docSection],
  };

  // A shape and a table, so their renderers are exercised rather than assumed.
  slides.push({
    ...core.makeSlide('blank', { title: '도형과 표', index: 3 }),
    title: '도형과 표',
    blocks: [
      (() => {
        const b = core.makeShape('flowChartDecision', { x: 96, y: 96, w: 300, h: 200, z: 1 });
        b.md = '승인?';
        return b;
      })(),
      core.makeTable(3, 3, { x: 500, y: 96, w: 600, h: 200, z: 2 }),
    ],
  });

  // A 4:3 deck: the size a projector deck is still made at, and the one every
  // hardcoded 16:9 assumption shows itself on.
  const standard = core.makeSlide('title-content', { title: '4:3 표준 비율' });
  fixtures['스모크-표준.aideck'] = {
    type: 'deck',
    folder: '스모크-표준.aideck',
    manifest: manifestOf('스모크 4:3', 'deck'),
    slides: [
      {
        ...standard,
        canvas: { w: 960, h: 720, bg: '#ffffff' },
        blocks: [
          ...standard.blocks,
          (() => {
            const b = core.makeShape('roundRect', { x: 672, y: 595, w: 250, h: 96, z: 9 });
            b.md = '우측 하단';
            return b;
          })(),
        ],
      },
    ],
  };

  // Two blocks stacked on top of each other, for the overlap UX checks:
  // Alt+click steps down the stack, the right-click menu lists what lies
  // underneath, and a block may be dragged past the canvas edge. Their z is
  // above the layout blocks so they are the ones hit at their point.
  fixtures['스모크-표준.aideck'].slides[0].blocks.push(
    {
      id: 'b_ux_back',
      kind: 'text',
      md: '뒤 블록',
      x: 300, y: 300, w: 300, h: 160, z: 50,
      style: { fontSize: 40, weight: 700, align: 'left', color: '#1f2937', lineHeight: 1.45 },
    },
    {
      id: 'b_ux_front',
      kind: 'text',
      md: '앞 블록',
      x: 300, y: 300, w: 300, h: 160, z: 51,
      style: { fontSize: 40, weight: 700, align: 'left', color: '#1f2937', lineHeight: 1.45 },
    }
  );

  // On disk a sheet is always saved recalculated; the in-memory fixture gets
  // the same treatment so the formula cells carry their cached values.
  const sample = core.makeSheet({ name: '시트1', withSample: true });
  const recalced = core.recalcSheet({ cells: sample.cells, names: sample.names, name: sample.name }, []);
  fixtures['스모크-시트.aigrid'] = {
    type: 'grid',
    folder: '스모크-시트.aigrid',
    manifest: manifestOf('스모크 grid', 'grid'),
    sheets: [{ ...sample, cells: recalced.cells }],
  };
}
const folders = Object.keys(fixtures);
// The fixtures live in memory; nothing is written to disk, so the workspace path
// the launcher displays is only a label.
const tmp = '/스모크';
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
    // Importing an Office file: the response shape the server sends, with the
    // conversion notes a real file produces.
    if (u.endsWith('/api/import')) {
      const folder = folders[0];
      return json({
        folder,
        project: fixtures[folder],
        warnings: [
          '화면에서는 글꼴을 Pretendard로 표시합니다 (Calibri) — 내보낼 때는 원래 글꼴 이름을 유지합니다',
          'SmartArt는 같은 모양의 도형들로 바꿨습니다',
          '조건부 서식은 저장된 값 기준의 고정 서식으로 바꿨습니다',
        ],
      });
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
  // Node ≥ 21 defines `globalThis.navigator` as a getter-only accessor, so a
  // plain assignment throws; redefining leaves it writable for the restore.
  Object.defineProperty(global, 'navigator', {
    value: window.navigator, configurable: true, writable: true,
  });
  // The bundle calls bare `fetch`, which resolves against globalThis rather than
  // the `window` we pass in, so the stub has to be installed globally too.
  global.fetch = window.fetch;
  // Reading a dropped file goes through `FileReader`, which the bundle reaches
  // as a bare identifier — so it has to exist on globalThis, not only on window.
  global.FileReader = window.FileReader;
  global.File = window.File;
  global.Blob = window.Blob;
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

  /**
   * Type into a rendered (contentEditable) surface.
   *
   * The paragraph editors are WYSIWYG: what a user types lands in the DOM and
   * the editor recovers markdown from it. The harness plays the user by
   * dropping the typed text into the surface as the browser would have it and
   * firing the input event the editor listens for.
   */
  const typeContent = (el, html) => {
    el.innerHTML = html;
    fire(el, 'input');
  };

  /** A `<select>` needs its own prototype's setter, and a `change` event. */
  const selectValue = (el, value) => {
    const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value').set;
    setter.call(el, value);
    fire(el, 'change');
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
    typeContent,
    selectValue,
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

// The first fixture of each type is the one the general checks run against;
// later ones (a 4:3 deck, say) are addressed by name.
const byType = Object.fromEntries([...folders].reverse().map((f) => [fixtures[f].type, f]));

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

console.log('\n■ Grid 동적 배열 스필');
{
  const captured = {};
  const { errors } = await mount(`#/grid/${encodeURIComponent(byType.grid)}`, {
    async interact({ window, settle, $, $$, click, key, setValue, cellText }) {
      // F2 (빈 셀)에 SEQUENCE(3)를 입력하면 F3·F4로 흘러넘친다.
      const rows = $$('.sheet tbody tr');
      const f2 = rows[1].querySelectorAll('td')[5];
      click(f2);
      await settle(4);
      key(window.document.body, '=');
      await settle(4);
      const editor = $('.cell__editor');
      if (!editor) throw new Error('typing = did not open the cell editor');
      setValue(editor, '=SEQUENCE(3)');
      key(editor, 'Enter');
      await settle(8);

      captured.anchor = cellText(2, 5);
      captured.spill2 = cellText(3, 5);
      captured.spill3 = cellText(4, 5);

      // 스필된 셀을 고르면 수식 입력줄이 앵커의 수식을 회색으로 보여준다.
      const f3 = $$('.sheet tbody tr')[2].querySelectorAll('td')[5];
      click(f3);
      await settle(4);
      captured.ghostFormula = $('.formulabar__input')?.value;
      captured.ghostClass = $('.formulabar__input')?.className ?? '';
      captured.outlined = $$('.sheet td.is-spill').length;
    },
  });

  check('스필 편집 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('앵커에 첫 값이 계산됨', captured.anchor?.includes('1'), `F2 = ${captured.anchor}`);
  check('배열이 아래 셀로 흘러넘침', captured.spill2?.includes('2') && captured.spill3?.includes('3'),
    `F3 = ${captured.spill2}, F4 = ${captured.spill3}`);
  check('스필된 셀의 수식 입력줄이 앵커의 수식을 보여줌',
    captured.ghostFormula === '=SEQUENCE(3)', `수식 입력줄 = ${captured.ghostFormula}`);
  check('그 수식은 회색(ghost)으로 표시됨', captured.ghostClass.includes('--ghost'), captured.ghostClass);
  check('스필 범위에 테두리가 그려짐', captured.outlined >= 2, `outlined = ${captured.outlined}`);
}

console.log('\n■ 인쇄 레이아웃 (Deck · Grid)');
{
  const captured = {};
  const deckMount = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    async interact({ window, settle, $, $$ }) {
      // 인쇄 레이아웃은 인쇄 중에만 마운트된다 — 화면 DOM을 복제하지 않는다.
      captured.deckBefore = $$('.printdeck').length;
      window.dispatchEvent(new window.Event('beforeprint'));
      await settle(2);
      captured.deckPages = $$('.printdeck__page').length;
      captured.deckSlides = $$('.slidesorter .sorter-item').length;
      window.dispatchEvent(new window.Event('afterprint'));
      await settle(2);
      captured.deckAfter = $$('.printdeck').length;
    },
  });
  check('인쇄 전에는 인쇄 레이아웃이 DOM에 없음', captured.deckBefore === 0);
  check('인쇄 중에는 슬라이드마다 한 페이지', captured.deckPages > 0 && captured.deckPages === captured.deckSlides,
    `pages=${captured.deckPages}, slides=${captured.deckSlides}`);
  check('인쇄가 끝나면 레이아웃이 내려감', captured.deckAfter === 0);
  check('덱 인쇄 중 런타임 오류 없음', deckMount.errors.length === 0, deckMount.errors.join('\n      '));

  const gridMount = await mount(`#/grid/${encodeURIComponent(byType.grid)}`, {
    async interact({ window, settle, $, $$ }) {
      window.dispatchEvent(new window.Event('beforeprint'));
      await settle(2);
      captured.gridCells = $$('.printsheet__table td').length;
      captured.gridText = $('.printsheet')?.textContent ?? '';
      window.dispatchEvent(new window.Event('afterprint'));
      await settle(2);
      captured.gridAfter = $$('.printsheet').length;
    },
  });
  check('시트 인쇄가 사용 범위를 표로 그림', captured.gridCells > 0, `cells=${captured.gridCells}`);
  check('인쇄된 시트에 계산된 값이 있음', captured.gridText.includes('2,550'), captured.gridText.slice(0, 120));
  check('시트 인쇄 레이아웃도 인쇄 후 내려감', captured.gridAfter === 0);
  check('시트 인쇄 중 런타임 오류 없음', gridMount.errors.length === 0, gridMount.errors.join('\n      '));
}

console.log('\n■ Deck 편집 상호작용');
{
  const captured = {};
  const { errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    async interact({ settle, $, $$, dblclick, typeContent }) {
      const block = $('.canvas .block');
      if (!block) throw new Error('no block on the canvas');
      dblclick(block);
      await settle(4);

      const editor = $('.block__editor');
      if (!editor) throw new Error('double-click did not open the block editor');
      typeContent(editor, '<p># 편집된 제목</p><p>두 번째 줄</p>');
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
    async interact({ settle, $, $$, click, typeContent }) {
      const blocks = $$('.docblock');
      if (!blocks.length) throw new Error('no blocks in the document');
      click(blocks[0]);
      await settle(4);

      const editor = $('.docblock__editor');
      if (!editor) throw new Error('clicking a paragraph did not open the editor');
      // The user types `## 새 소제목` at the start of an empty paragraph; the
      // editor recovers the markdown from the drawn text.
      typeContent(editor, '<p>## 새 소제목</p>');
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
  const anchors = (captured.md.match(/<!-- block:/g) ?? []).length;
  check('서식을 준 문단만 md에 앵커가 생김',
    captured.md ? anchors === 1 && captured.md.includes('b_docstyled') : false,
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
  check('방향키로 슬라이드 이동', got.counter === '2 / 3', `counter = ${got.counter}`);
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

console.log('\n■ Doc 문단 편집 (Word의 손버릇)');
{
  const got = {};
  const { errors } = await mount(`#/doc/${encodeURIComponent(byType.doc)}`, {
    async interact({ settle, $, $$, click, fire, typeContent }) {
      const block = $$('.docblock')[0];
      click(block);
      await settle(6);
      const ta = $('.docblock__editor');
      got.opened = !!ta;
      if (!ta) return;

      /*
       * The paragraph is its own edit surface now: the typed text lands in the
       * drawn form, and a formatting shortcut must not throw the writer out of
       * it. If the surface were remounted on re-render, focus (and with it the
       * caret) would be lost — so focus staying put is the check.
       */
      typeContent(ta, '<p>가나다라마</p>');
      await settle(6);
      fire(ta, 'keydown', { key: 'l', ctrlKey: true });
      await settle(8);
      got.stillEditing = $('.docblock.is-editing') === ta.parentElement;
      got.stillFocused = document.activeElement === ta;
      got.textAfterAlign = ta.textContent;

      // Tab indents rather than throwing focus out of the document.
      typeContent(ta, '<p>- 항목</p>');
      await settle(6);
      fire(ta, 'keydown', { key: 'Tab' });
      await settle(8);
      const mdTab = $$('.inspector__tab').find((t) => t.textContent.endsWith('.md'));
      if (mdTab) {
        click(mdTab);
        await settle(4);
        got.mdAfterTab = $('.code')?.textContent ?? '';
      }

      // Ctrl+U underlines: markdown has no syntax for it, so it has to land in
      // the paragraph's meta.json.
      fire(ta, 'keydown', { key: 'u', ctrlKey: true });
      await settle(8);
      got.underlineBadge = !!$('.docblock__badge');
      const jsonTab = $$('.inspector__tab').find((t) => t.textContent.includes('.meta.json'));
      if (jsonTab) click(jsonTab);
      await settle(5);
      got.metaHasUnderline = ($('.panel--wide .code')?.textContent ?? '').includes('underline');
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('문단을 누르면 문단 자체가 편집면이 됨', got.opened === true);
  check('서식 단축키 후에도 문단에서 계속 쓸 수 있음',
    got.stillEditing === true && got.stillFocused === true,
    `editing=${got.stillEditing} focused=${got.stillFocused}`);
  check('맞춤을 바꿔도 문단 내용이 그대로임', got.textAfterAlign?.includes('가나다라마'), got.textAfterAlign);
  check('Tab이 목록 수준을 내림', got.mdAfterTab?.includes('  - 항목'), JSON.stringify(got.mdAfterTab));
  check('Ctrl+U가 문단 서식으로 기록됨', got.underlineBadge === true);
  check('밑줄이 meta.json에 들어감', got.metaHasUnderline === true);
}

console.log('\n■ Doc 자유로운 문단 편집 UX');
{
  const got = {};
  const { errors } = await mount(`#/doc/${encodeURIComponent(byType.doc)}`, {
    async interact({ window, settle, $, $$, click, dblclick }) {
      // A paragraph with its own size keeps it while being edited: the drawn
      // text is the edit surface, so the type the user sees while fixing a
      // sentence is the type the reader will see.
      const styled = $$('.docblock').find((b) => b.textContent.includes('큰 글씨 문단'));
      click(styled);
      await settle(6);
      const wrapper = $('.docblock.is-editing');
      got.editorFontSize = wrapper?.style?.fontSize;
      got.sizedClass = wrapper?.querySelector('.docblock__editor')?.className ?? '';
      got.editingClass = !!wrapper;

      // Double-clicking open page space starts a new paragraph at the end of
      // the page, ready to type — no ribbon or menu involved.
      got.before = $$('.docblock').length;
      const inner = $('.page__inner');
      dblclick(inner);
      await settle(8);
      got.docsAfter = $$('.docblock').length;
      got.newParagraphEditing =
        !!$('.docblock.is-editing') && !!$('.docblock.is-editing .docblock__editor');
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('문단 글꼴 크기가 편집 중에도 유지됨', got.editorFontSize === '22px', `fontSize = ${got.editorFontSize}`);
  check('편집면이 문단 서식 클래스를 물려받음', got.sizedClass?.includes('md--sized'), got.sizedClass);
  check('편집 중임이 표시됨', got.editingClass === true);
  check('빈 영역 더블클릭으로 문단 추가', got.docsAfter === got.before + 1, `blocks ${got.before} → ${got.docsAfter}`);
  check('새 문단이 바로 편집 모드로 열림', got.newParagraphEditing === true);
}

console.log('\n■ Grid 이동 (Excel의 손버릇)');
{
  const got = {};
  const { errors } = await mount(`#/grid/${encodeURIComponent(byType.grid)}`, {
    async interact({ settle, $, $$, winKey, cellText }) {
      const label = () => $('.formulabar__ref')?.value;

      // Ctrl+Down runs to the end of the data instead of moving one row.
      winKey('ArrowDown', { ctrlKey: true });
      await settle(5);
      got.afterCtrlDown = label();

      winKey('Home', { ctrlKey: true });
      await settle(4);
      got.afterCtrlHome = label();

      // Ctrl+Shift+Down selects the column of values, not one extra cell.
      winKey('ArrowDown', { ctrlKey: true, shiftKey: true });
      await settle(5);
      got.afterCtrlShiftDown = label();

      // Ctrl+End goes to the last used cell of the sheet.
      winKey('End', { ctrlKey: true });
      await settle(4);
      got.afterCtrlEnd = label();

      // Ctrl+A takes the block the cursor is in.
      winKey('Home', { ctrlKey: true });
      await settle(3);
      winKey('a', { ctrlKey: true });
      await settle(4);
      got.afterCtrlA = label();

      // Ctrl+Space is the whole column, Shift+Space the whole row.
      winKey('Home', { ctrlKey: true });
      await settle(3);
      winKey(' ', { ctrlKey: true });
      await settle(4);
      got.afterCtrlSpace = label();

      // Ctrl+; stamps today's date into the cell.
      winKey('Home', { ctrlKey: true });
      await settle(3);
      winKey('ArrowDown');
      winKey('ArrowDown');
      winKey('ArrowDown');
      winKey('ArrowDown');
      winKey('ArrowDown');
      winKey('ArrowDown');
      winKey('ArrowDown');
      winKey('ArrowDown');
      winKey('ArrowDown');
      winKey('ArrowDown');
      winKey('ArrowDown');
      winKey('ArrowDown');
      await settle(5);
      got.dateCellRef = label();
      winKey(';', { ctrlKey: true });
      await settle(8);
      got.stamped = $('.formulabar__input')?.value;

      // Ctrl+Enter fills the selection with the active cell's value rather
      // than opening the cell for editing.
      winKey('Home', { ctrlKey: true });
      await settle(3);
      winKey('ArrowDown', { ctrlKey: true, shiftKey: true });
      await settle(4);
      winKey('Enter', { ctrlKey: true });
      await settle(10);
      got.filledSame = cellText(1, 0) === cellText(2, 0) && cellText(1, 0) !== '';
      got.stillNotEditing = !$('.cell__editor');

      // Ctrl+1 opens 셀 서식.
      winKey('1', { ctrlKey: true });
      await settle(5);
      got.formatDialog = ($('.dialog__head')?.textContent ?? '').includes('셀 서식');
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('Ctrl+↓가 데이터 끝으로 건너뜀', /^A[2-9]|^A\d\d/.test(got.afterCtrlDown ?? ''), `ref = ${got.afterCtrlDown}`);
  check('Ctrl+Home이 A1로', got.afterCtrlHome === 'A1', `ref = ${got.afterCtrlHome}`);
  check('Ctrl+Shift+↓가 범위를 끝까지 확장', /^A1:A\d+$/.test(got.afterCtrlShiftDown ?? ''), `ref = ${got.afterCtrlShiftDown}`);
  check('Ctrl+End가 마지막 데이터 셀로', /^[B-Z]\d+$/.test(got.afterCtrlEnd ?? ''), `ref = ${got.afterCtrlEnd}`);
  check('Ctrl+A가 데이터 블록을 선택', /^A1:[A-Z]+\d+$/.test(got.afterCtrlA ?? ''), `ref = ${got.afterCtrlA}`);
  check('Ctrl+Space가 열 전체를 선택', /^A1:A\d{2,}$/.test(got.afterCtrlSpace ?? ''), `ref = ${got.afterCtrlSpace}`);
  check('Ctrl+;가 오늘 날짜를 넣음', /^\d{4}-\d{2}-\d{2}$/.test(got.stamped ?? ''), `${got.dateCellRef} = ${got.stamped}`);
  check('Ctrl+Enter가 선택 영역을 같은 값으로 채움', got.filledSame === true);
  check('Ctrl+Enter가 셀 편집으로 새지 않음', got.stillNotEditing === true);
  check('Ctrl+1이 셀 서식을 열음', got.formatDialog === true);
}

console.log('\n■ 슬라이드 쇼 (보이는 것이 만든 것과 같은지)');
{
  const got = {};
  const { errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    async interact({ settle, $, $$, click, winKey }) {
      // Go to the slide that has a real shape and a real table on it.
      const thumbs = $$('.sorter-item');
      got.slides = thumbs.length;
      if (thumbs[2]) click(thumbs[2]);
      await settle(6);
      got.shapesOnCanvas = $$('.canvas svg.shape').length;

      // Shift+F5 presents the slide being looked at, so the shape and table on
      // slide 3 are the ones on screen.
      winKey('F5', { shiftKey: true });
      await settle(8);
      got.showing = !!$('.slideshow');
      got.fromCurrent = $('.slideshow__count')?.textContent?.trim();
      // The shape must be drawn as its preset geometry here too.
      got.shapesInShow = $$('.slideshow svg.shape').length;
      got.tablesInShow = $$('.slideshow table').length;

      // B blanks the screen; pressing it again brings the slide back.
      winKey('b');
      await settle(4);
      got.blanked = !!$('.slideshow__blank');
      winKey('b');
      await settle(4);
      got.unblanked = !$('.slideshow__blank');

      winKey('Escape');
      await settle(5);
      got.closed = !$('.slideshow');

      // A bare F5 starts at the top, as its own ribbon label promises.
      winKey('F5');
      await settle(8);
      got.showCounter = $('.slideshow__count')?.textContent?.trim();
      winKey('Escape');
      await settle(4);
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('캔버스에 도형이 프리셋 도형으로 그려짐', (got.shapesOnCanvas ?? 0) > 0, `${got.shapesOnCanvas}`);
  check('Shift+F5가 슬라이드 쇼를 시작', got.showing === true);
  check('Shift+F5는 현재 슬라이드부터', /^3 \//.test(got.fromCurrent ?? ''), `counter = ${got.fromCurrent}`);
  check('F5는 처음 슬라이드부터', /^1 \//.test(got.showCounter ?? ''), `counter = ${got.showCounter}`);
  check(
    '쇼에서도 도형이 사각형이 아니라 그 도형으로 보임',
    (got.shapesInShow ?? 0) === (got.shapesOnCanvas ?? -1),
    `캔버스 ${got.shapesOnCanvas}개 / 쇼 ${got.shapesInShow}개`
  );
  check('쇼에서 표가 표로 그려짐', (got.tablesInShow ?? 0) > 0, `${got.tablesInShow}`);
  check('B가 화면을 지움', got.blanked === true);
  check('B를 다시 누르면 돌아옴', got.unblanked === true);
  check('Esc로 쇼가 끝남', got.closed === true);
}

console.log('\n■ 표 안의 키가 슬라이드를 건드리지 않는지 (Deck)');
{
  const got = {};
  const { errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    async interact({ settle, $, $$, click, fire, winKey }) {
      const thumbs = $$('.sorter-item');
      if (thumbs[2]) click(thumbs[2]);
      await settle(6);
      got.blocksBefore = $$('.canvas .block').length;

      // Put the cursor in a table cell — the cell listens on pointerdown —
      // then press the keys the canvas also listens for on window.
      const cells = $$('.canvas .tableblock td, .canvas .tableblock th');
      const cell = cells[1] ?? cells[0];
      if (cell) fire(cell, 'pointerdown', { button: 0 });
      await settle(8);
      got.cursorInCell = !!$('.canvas .tableblock .is-active, .canvas .tableblock td.is-active, .canvas .tableblock th.is-active');
      got.textBefore = cell?.textContent ?? '';

      // The arrow first, while the table is certainly still there: if the
      // canvas also handles it the table slides sideways under the cursor.
      const box = () => {
        const el = $$('.canvas .block').find((b) => b.querySelector('.tableblock'));
        return el ? `${el.style.left},${el.style.top}` : null;
      };
      const before = box();
      got.foundTableBlock = before !== null;
      winKey('ArrowRight');
      await settle(8);
      got.moved = box() !== before;

      winKey('Delete');
      await settle(10);
      got.blocksAfterDelete = $$('.canvas .block').length;
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('셀에 커서가 들어감', got.cursorInCell === true);
  check('표 블록을 찾음', got.foundTableBlock === true);
  check(
    '셀에 커서가 있을 때 Delete가 표를 지우지 않음',
    got.blocksAfterDelete === got.blocksBefore,
    `${got.blocksBefore} → ${got.blocksAfterDelete}`
  );
  check('셀에 커서가 있을 때 화살표가 표를 끌고 가지 않음', got.moved === false);
}

console.log('\n■ 서식 복사 (Office에서 가장 많이 누르는 버튼)');
{
  const got = {};
  const { errors } = await mount(`#/grid/${encodeURIComponent(byType.grid)}`, {
    async interact({ settle, $, $$, click, winKey }) {
      // Make A1 bold, pick its format up, and paint it onto A3.
      winKey('Home', { ctrlKey: true });
      await settle(4);
      winKey('b', { ctrlKey: true });
      await settle(6);

      const brush = () => $$('.rbtn').find((b) => /서식 복사|여기에 붙이기/.test(b.textContent));
      got.hasBrush = !!brush();
      click(brush());
      await settle(5);
      got.armed = /여기에 붙이기/.test(brush()?.textContent ?? '');

      winKey('ArrowDown');
      winKey('ArrowDown');
      await settle(5);
      click(brush());
      await settle(8);
      got.disarmed = /서식 복사/.test(brush()?.textContent ?? '');
      const json = $$('.inspector__tab').find((t) => t.textContent.includes('.cells.json'));
      if (json) click(json);
      await settle(5);
      got.cells = $('.panel--wide .code')?.textContent ?? '';
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('클립보드 그룹에 서식 복사가 있음', got.hasBrush === true);
  check('누르면 붙일 준비 상태가 됨', got.armed === true);
  check('붙이면 다시 풀림', got.disarmed === true);
  check('서식이 옮겨져 JSON에 기록됨', (got.cells.match(/"bold": true/g) ?? []).length >= 2, sample(got.cells));
}

console.log('\n■ 단축키 도움말 (F1)');
{
  for (const [type, expect] of [['deck', 'Ctrl+M'], ['doc', 'Ctrl+U'], ['grid', 'Alt+Enter']]) {
    const got = {};
    const { errors } = await mount(`#/${type}/${encodeURIComponent(byType[type])}`, {
      async interact({ settle, $, winKey }) {
        winKey('F1');
        await settle(6);
        got.open = !!$('.shortcuts');
        got.text = $('.shortcuts')?.textContent ?? '';
        winKey('Escape');
        await settle(4);
        got.closed = !$('.shortcuts');
      },
    });
    check(`${type}: F1이 단축키 목록을 열음`, got.open === true, errors.join('\n      '));
    check(`${type}: 이 앱의 단축키가 실려 있음 (${expect})`, got.text.includes(expect), sample(got.text));
    check(`${type}: Esc로 닫힘`, got.closed === true);
  }
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


console.log('\n■ 도형 · 표 렌더링');
{
  const { html, text, errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    interact: async ({ window, settle }) => {
      // The third slide is the one with a shape and a table.
      const thumbs = [...window.document.querySelectorAll('.sorter-item')];
      if (thumbs[2]) {
        thumbs[2].dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
        await settle(6);
      }
    },
  });
  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('도형이 SVG 경로로 그려짐', /class="shape"/.test(html) && /<path/.test(html), html.slice(0, 200));
  check('도형 안의 텍스트가 보임', text.includes('승인?'));
  check('표가 실제 table 요소로 그려짐', /class="tableblock/.test(html));
  check('표에 머리글 셀이 있음', /<th/.test(html));
}

console.log('\n■ 삽입 리본 (도형 갤러리 · 표 격자)');
{
  const { errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    interact: async ({ window, settle, $, $$, click, fire }) => {
      const insertTab = $$('.ribbon__tab, .tabs button').find((t) => t.textContent.trim() === '삽입');
      if (insertTab) {
        click(insertTab);
        await settle(4);
      }
      const shapeBtn = $$('button').find((b) => b.textContent.includes('도형'));
      check('삽입 탭에 도형 버튼이 있음', !!shapeBtn);
      if (shapeBtn) {
        click(shapeBtn);
        await settle(4);
        const gallery = $('.gallery');
        check('도형 갤러리가 열림', !!gallery);
        check(
          '갤러리가 Office 서랍 이름을 씀',
          !!gallery && gallery.textContent.includes('순서도') && gallery.textContent.includes('블록 화살표'),
          gallery?.textContent.slice(0, 120)
        );
        const items = $$('.gallery__item');
        check('갤러리에 도형이 100개 이상', items.length > 100, `${items.length}개`);
        if (items[0]) {
          const before = $$('.shape').length;
          click(items[0]);
          await settle(4);
          // Office arms the cursor rather than inserting: the shape appears when
          // you drag (or click) on the slide.
          check('도형을 고르면 갤러리가 닫힘', !$('.gallery'));
          const canvas = $('.canvas');
          check('캔버스가 십자 커서로 바뀜', canvas?.style.cursor === 'crosshair', canvas?.style.cursor);
          if (canvas) {
            fire(canvas, 'pointerdown', { button: 0, pointerId: 1, clientX: 300, clientY: 220 });
            await settle(2);
            fire(canvas, 'pointermove', { pointerId: 1, clientX: 480, clientY: 340 });
            await settle(2);
            fire(canvas, 'pointerup', { pointerId: 1, clientX: 480, clientY: 340 });
            await settle(6);
            check('캔버스에 끌어 그리면 삽입됨', $$('.shape').length > before);
            check('삽입 후 도형 서식 탭이 열림', !!$$('.ribbon__tab').find(
              (t) => t.textContent.trim() === '도형 서식' && t.getAttribute('aria-selected') === 'true'
            ));
            check('회전 핸들이 나타남', !!$('.handle--rotate'));
          }
        }
      }

      const tableBtn = $$('button').find((b) => b.textContent.trim() === '표');
      if (tableBtn) {
        click(tableBtn);
        await settle(4);
        const picker = $('.tablepicker');
        check('표 격자 선택기가 열림', !!picker);
        const cells = $$('.tablepicker__cell');
        check('격자가 10×8', cells.length === 80, `${cells.length}개`);
        if (cells[12]) {
          click(cells[12]);
          await settle(4);
          check('격자를 클릭하면 표가 삽입됨', $$('.tableblock').length > 0);
        }
      }
    },
  });
  check('삽입 상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
}

console.log('\n■ 상황별 탭 (표 도구 · 도형 도구)');
{
  const captured = {};
  const { errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    interact: async ({ settle, $, $$, click, fire }) => {
      // Slide 3 of the fixture holds a shape and a table.
      const thumbs = $$('.sorter-item');
      if (thumbs[2]) {
        click(thumbs[2]);
        await settle(6);
      }

      const shape = $('.block.is-selected') ?? $$('.block')[0];
      if (shape) {
        fire(shape, 'pointerdown', { button: 0, pointerId: 2 });
        await settle(2);
        fire(shape, 'pointerup', { pointerId: 2 });
        await settle(4);
        captured.shapeTabs = $$('.ribbon__tab--context').map((t) => t.textContent.trim());
        captured.contextLabel = $('.ribbon__contextlabel')?.textContent;
        const format = $$('.ribbon__tab').find((t) => t.textContent.trim() === '도형 서식');
        if (format) {
          click(format);
          await settle(4);
          const body = $('.ribbon__body')?.textContent ?? '';
          captured.shapeGroups = ['도형 채우기', '도형 윤곽선', '회전'].filter((g) => body.includes(g));
          // The fill control is Office's split button: a swatch that opens a
          // palette of theme colours, standard colours, "no fill" and a picker.
          const fill = $$('.colorbtn').find((b) => b.getAttribute('aria-label') === '채우기 색');
          if (fill) {
            click(fill);
            await settle(4);
            captured.paletteSwatches = $$('.palette .swatch').length;
            captured.paletteLabels = $$('.palette__label').map((l) => l.textContent.trim());
            captured.hasCustom = !!$('.palette input[type="color"]');
            const shapePath = () => $$('svg path').map((el) => el.getAttribute('fill')).find((f) => f);
            const before = shapePath();
            const noFill = $$('.palette__item').find((b) => b.textContent.includes('채우기 없음'));
            if (noFill) {
              click(noFill);
              await settle(6);
              const after = shapePath();
              captured.noFillWorked = before !== after && after === 'none';
              captured.fillBefore = before;
              captured.fillAfter = after;
            }
          }
        }
      }

      // Now the table: click a cell, then use the Layout tab.
      const cell = $$('.tableblock td, .tableblock th')[0];
      if (cell) {
        fire(cell, 'pointerdown', { button: 0, pointerId: 3 });
        await settle(4);
        captured.cellActive = !!$('.tableblock .is-active');
        captured.tableTabs = $$('.ribbon__tab--context').map((t) => t.textContent.trim());

        const layout = $$('.ribbon__tab').find((t) => t.textContent.trim() === '레이아웃');
        if (layout) {
          click(layout);
          await settle(4);
          const rowsBefore = $$('.tableblock tr').length;
          const insertBelow = $$('button').find((b) => b.textContent.includes('아래에 삽입'));
          if (insertBelow) {
            click(insertBelow);
            await settle(6);
            captured.rowAdded = $$('.tableblock tr').length === rowsBefore + 1;
          }
          const mergeRight = $$('button').find((b) => b.textContent.includes('오른쪽과 병합'));
          if (mergeRight) {
            click(mergeRight);
            await settle(6);
            captured.merged = !!$$('.tableblock td[colspan], .tableblock th[colspan]').length;
          }
        }
      }
    },
  });

  check('상황별 탭 상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('도형을 고르면 도형 서식 탭이 붙음', captured.shapeTabs?.includes('도형 서식'),
    JSON.stringify(captured.shapeTabs));
  check('상황별 탭에 대상 이름이 붙음', captured.contextLabel === '도형 도구', captured.contextLabel);
  check('도형 서식 탭에 Office 그룹이 있음', captured.shapeGroups?.length === 3,
    JSON.stringify(captured.shapeGroups));
  check('채우기 색이 Office식 팔레트로 열림',
    captured.paletteSwatches === 70 && JSON.stringify(captured.paletteLabels) === JSON.stringify(['테마 색', '표준 색']),
    `${captured.paletteSwatches} swatches · ${JSON.stringify(captured.paletteLabels)}`);
  check('팔레트에 다른 색 선택이 있음', captured.hasCustom === true);
  check('채우기 없음이 도형에 반영됨', captured.noFillWorked === true,
    `${captured.fillBefore} → ${captured.fillAfter}`);
  check('셀을 누르면 선택 표시됨', captured.cellActive === true);
  check('표를 고르면 표 도구 탭이 붙음',
    captured.tableTabs?.includes('표 디자인') && captured.tableTabs?.includes('레이아웃'),
    JSON.stringify(captured.tableTabs));
  check('레이아웃 탭에서 행을 삽입', captured.rowAdded === true);
  check('레이아웃 탭에서 셀을 병합', captured.merged === true);
}

console.log('\n■ 표 셀 편집 (Office 방식)');
{
  const captured = {};
  const { errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    interact: async ({ settle, $, $$, click, fire, setValue, fireWindow }) => {
      const thumbs = $$('.sorter-item');
      if (thumbs[2]) {
        click(thumbs[2]);
        await settle(6);
      }
      const cells = $$('.tableblock td, .tableblock th');
      if (!cells.length) return;

      // Double-click opens the in-cell editor, as in Office.
      fire(cells[0], 'pointerdown', { button: 0, pointerId: 4 });
      await settle(3);
      fire(cells[0], 'dblclick', {});
      await settle(6);
      const editor = $('.tableblock__editor');
      captured.opened = !!editor;
      if (editor) {
        setValue(editor, '입력한 값');
        // Tab commits and moves to the next cell.
        fire(editor, 'keydown', { key: 'Tab' });
        await settle(8);
        captured.committed = ($('.tableblock')?.textContent ?? '').includes('입력한 값');
        captured.movedOn = !!$('.tableblock__editor');
      }
      // Escape leaves editing without losing the committed text.
      fireWindow('keydown', { key: 'Escape' });
      await settle(4);
    },
  });

  check('셀 편집 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('두 번 누르면 셀 편집기가 열림', captured.opened === true);
  check('입력한 값이 표에 반영됨', captured.committed === true);
  check('Tab이 다음 셀로 이동', captured.movedOn === true);
}

console.log('\n■ 리본 구성 (Office 이름과 순서)');
{
  const expected = {
    deck: {
      tabs: ['파일', '홈', '삽입', '디자인', '슬라이드 쇼', 'AI'],
      홈: ['되돌리기', '클립보드', '슬라이드', '글꼴', '색', '단락', '그리기'],
      삽입: ['표', '이미지', '일러스트레이션', '텍스트'],
    },
    doc: {
      tabs: ['파일', '홈', '삽입', '레이아웃', '보기', 'AI'],
      // Word's own order in this tab: margins, orientation, size.
      레이아웃: ['여백 (px)', '용지 방향', '용지 크기', '단', '크기 (px)', '섹션 이름'],
    },
    grid: {
      tabs: ['파일', '홈', '삽입', '수식', '데이터', 'AI'],
    },
  };

  for (const [type, want] of Object.entries(expected)) {
    const { errors } = await mount(`#/${type}/${encodeURIComponent(byType[type])}`, {
      interact: async ({ settle, $, $$, click }) => {
        const tabs = $$('.ribbon__tab:not(.ribbon__tab--context)').map((t) => t.textContent.trim());
        check(`${type}: 탭 이름과 순서`, JSON.stringify(tabs) === JSON.stringify(want.tabs),
          JSON.stringify(tabs));

        for (const [tabName, groups] of Object.entries(want)) {
          if (tabName === 'tabs') continue;
          const button = $$('.ribbon__tab').find((t) => t.textContent.trim() === tabName);
          if (!button) continue;
          click(button);
          await settle(4);
          const found = $$('.ribbon__body .rgroup__label').map((l) => l.textContent.trim());
          check(`${type} ${tabName}: 그룹 순서`,
            JSON.stringify(found) === JSON.stringify(groups),
            JSON.stringify(found));
        }
      },
    });
    check(`${type}: 리본 렌더링에 오류 없음`, errors.length === 0, errors.join('\n      '));
  }
}

console.log('\n■ Doc 표 도구');
{
  const captured = {};
  const { errors } = await mount(`#/doc/${encodeURIComponent(byType.doc)}`, {
    interact: async ({ settle, $, $$, click, fire }) => {
      captured.rendered = $$('.tableblock').length > 0;
      const cell = $$('.tableblock td, .tableblock th')[0];
      if (!cell) return;
      fire(cell, 'pointerdown', { button: 0, pointerId: 7 });
      await settle(4);
      captured.tabs = $$('.ribbon__tab--context').map((t) => t.textContent.trim());
      captured.label = $('.ribbon__contextlabel')?.textContent;
      // A table must not host the text edit surface — its cells are edited
      // through the table's own grid, not as markdown.
      captured.noTextarea = !$('.tableblock .docblock__editor');

      const layout = $$('.ribbon__tab').find((t) => t.textContent.trim() === '표 레이아웃');
      if (layout) {
        click(layout);
        await settle(4);
        const before = $$('.tableblock tr').length;
        const insertRight = $$('button').find((b) => b.textContent.includes('오른쪽에 삽입'));
        const cols = $$('.tableblock tr')[0]?.children.length;
        if (insertRight) {
          click(insertRight);
          await settle(6);
          captured.colAdded = $$('.tableblock tr')[0]?.children.length === cols + 1;
          captured.rowsUnchanged = $$('.tableblock tr').length === before;
        }
      }
    },
  });

  check('Doc 표 상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('Doc의 표가 table 요소로 그려짐', captured.rendered === true);
  check('표를 고르면 표 도구 탭이 붙음',
    captured.tabs?.includes('표 디자인') && captured.tabs?.includes('표 레이아웃'),
    JSON.stringify(captured.tabs));
  check('상황별 탭에 대상 이름이 붙음', captured.label === '표 도구', captured.label);
  check('표는 마크다운 편집기를 열지 않음', captured.noTextarea === true);
  check('열을 삽입해도 행 수는 그대로', captured.colAdded === true && captured.rowsUnchanged === true,
    `colAdded=${captured.colAdded} rowsUnchanged=${captured.rowsUnchanged}`);
}

console.log('\n■ 용지 크기와 방향 (Doc)');
{
  const { errors } = await mount(`#/doc/${encodeURIComponent(byType.doc)}`, {
    interact: async ({ settle, $, $$, click, setValue, selectValue }) => {
      const layout = $$('.ribbon__tab').find((t) => t.textContent.trim() === '레이아웃');
      click(layout);
      await settle(4);

      const page = () => $('.page');
      const size = () => ({ w: Math.round(page().getBoundingClientRect().width || parseFloat(page().style.width)),
                            h: Math.round(parseFloat(page().style.minHeight)) });
      const before = size();
      check('A4 세로로 시작', before.w === 794 && before.h === 1123, JSON.stringify(before));

      // Office keeps the size list and the orientation as separate controls, so
      // turning the page must not reset the paper.
      const landscape = $$('button').find((b) => b.textContent.includes('가로'));
      click(landscape);
      await settle(6);
      const turned = size();
      check('가로로 바꾸면 비율이 뒤바뀜', turned.w === 1123 && turned.h === 794, JSON.stringify(turned));

      const select = $$('select').find((el) => el.getAttribute('title') === '용지 크기');
      check('용지 크기 목록에 A4가 있음', !!select && [...select.options].some((o) => o.value === 'A4'));
      check('용지 크기는 여전히 A4', select.value === 'A4', select?.value);

      // A4 chosen while landscape stays landscape, as in Word.
      selectValue(select, 'Letter');
      await settle(6);
      const letter = size();
      check('가로 상태에서 Letter를 고르면 가로 Letter', letter.w === 1056 && letter.h === 816,
        JSON.stringify(letter));

      const width = $$('input[type="number"]').find((el) => el.getAttribute('title') === '너비');
      check('너비를 직접 입력할 수 있음', !!width);
      setValue(width, '700');
      await settle(6);
      const custom = size();
      check('임의 크기가 그대로 적용됨', custom.w === 700, JSON.stringify(custom));
      const options = [...select.options].map((o) => o.value);
      check('이름 없는 크기는 사용자 지정으로 표시', select.value === '사용자 지정' && options.includes('사용자 지정'),
        `${select.value} / ${options.join(',')}`);
    },
  });
  check('용지 설정에 오류 없음', errors.length === 0, errors.join('\n      '));
}

console.log('\n■ 슬라이드 크기 (Deck)');
{
  const { errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    interact: async ({ settle, $, $$, click, selectValue }) => {
      const design = $$('.ribbon__tab').find((t) => t.textContent.trim() === '디자인');
      click(design);
      await settle(4);
      const select = $$('select').find((el) => el.getAttribute('title') === '슬라이드 크기');
      check('슬라이드 크기 목록이 있음', !!select);
      const labels = [...select.options].map((o) => o.textContent.trim());
      check('PowerPoint의 두 크기를 제공', labels.includes('와이드스크린 (16:9)') && labels.includes('표준 (4:3)'),
        labels.join(', '));
      check('현재 크기가 선택되어 있음', select.value === '1280x720', select.value);

      selectValue(select, '960x720');
      await settle(6);
      const canvas = $('.canvas');
      check('4:3으로 바꾸면 캔버스가 좁아짐', Math.round(parseFloat(canvas.style.width)) === 960,
        canvas.style.width);
    },
  });
  check('슬라이드 크기 변경에 오류 없음', errors.length === 0, errors.join('\n      '));
}

console.log('\n■ 두 단 문서 (Doc)');
{
  const { errors } = await mount(`#/doc/${encodeURIComponent(byType.doc)}`, {
    interact: async ({ settle, $, $$, click, selectValue }) => {
      const layout = $$('.ribbon__tab').find((t) => t.textContent.trim() === '레이아웃');
      click(layout);
      await settle(4);
      const select = $$('select').find((el) => el.getAttribute('title') === '단');
      check('단 설정이 있음', !!select);
      selectValue(select, '2');
      await settle(8);
      const inner = $('.page__inner');
      check('페이지가 두 단으로 흐름', inner?.style.columnCount === '2', inner?.style.columnCount);
      check('단 사이 간격이 Word와 같음', parseFloat(inner?.style.columnGap) === 48, inner?.style.columnGap);
    },
  });
  check('단 설정에 오류 없음', errors.length === 0, errors.join('\n      '));
}

console.log('\n■ 수식 리본 (함수 목록)');
{
  const { errors } = await mount(`#/grid/${encodeURIComponent(byType.grid)}`, {
    interact: async ({ settle, $$, click, selectValue }) => {
      const formulas = $$('.ribbon__tab').find((t) => t.textContent.trim() === '수식');
      click(formulas);
      await settle(4);

      const groups = $$('.ribbon__body .rgroup__label').map((l) => l.textContent.trim());
      check('Excel의 함수 범주를 그 순서로', 
        JSON.stringify(groups) ===
          JSON.stringify(['재무', '논리', '텍스트', '날짜 및 시간', '찾기/참조', '수학/삼각', '통계', '검사']),
        groups.join(', '));

      // Every name the menus offer has to exist in the engine.
      const offered = $$('.ribbon__body select')
        .flatMap((el) => [...el.options].map((o) => o.value))
        .filter((v) => v);
      const missing = offered.filter((name) => !core.FUNCTION_NAMES.includes(name));
      check(`목록의 함수 ${offered.length}개가 모두 구현됨`, missing.length === 0, missing.join(', '));
      check('엔진에 함수가 200개 이상', core.FUNCTION_NAMES.length >= 200, String(core.FUNCTION_NAMES.length));

      // And picking one starts the formula.
      const financial = $$('.ribbon__body select')[0];
      selectValue(financial, 'PMT');
      await settle(6);
      check(
        '고르면 수식 입력이 시작됨',
        !!$$('input, textarea').find((el) => String(el.value).startsWith('=PMT(')),
        $$('.formulabar input').map((el) => el.value).join('|')
      );
    },
  });
  check('수식 리본에 오류 없음', errors.length === 0, errors.join('\n      '));
}

console.log('\n■ 리본을 넘어가는 메뉴 (잘림 방지)');
{
  // The ribbon scrolls sideways, so it clips its own children. A palette or a
  // shape gallery placed inside it was cut off at the ribbon's edge; they hang
  // off the control from `document.body` instead.
  const { errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    interact: async ({ settle, $, $$, click, fire, winKey }) => {
      const insert = $$('.ribbon__tab').find((t) => t.textContent.trim() === '삽입');
      click(insert);
      await settle(4);

      const inRibbon = (el) => !!el?.closest('.ribbon');
      const parentIsBody = (el) => el?.parentElement === document.body;

      // The shape gallery.
      click($$('button').find((b) => b.textContent.includes('도형')));
      await settle(6);
      const gallery = $('.gallery');
      check('도형 갤러리가 열림', !!gallery);
      check('갤러리가 리본 밖에 그려짐', !inRibbon(gallery), gallery?.parentElement?.className);
      check('갤러리가 body 직속의 팝오버 안에', parentIsBody(gallery?.closest('.popover')));
      check('팝오버는 fixed로 놓임', $('.popover')?.style.position !== 'absolute');
      winKey('Escape');
      await settle(4);
      check('Esc로 갤러리가 닫힘', !$('.gallery'));

      // The table grid.
      // The icon is part of the label text, so the match is on the ending.
      const tableButton = $$('.rbtn').find((b) => b.textContent.trim().endsWith('표'));
      check('표 단추가 있음', !!tableButton);
      click(tableButton);
      await settle(6);
      check('표 격자도 리본 밖에', !inRibbon($('.tablepicker')) && !!$('.tablepicker'));

      // A click somewhere else closes it, as a menu does.
      const away = $('.canvas') ?? $('.stage') ?? document.body;
      away.dispatchEvent(new window.MouseEvent('mousedown', { bubbles: true }));
      await settle(4);
      check('바깥을 누르면 닫힘', !$('.tablepicker'));

      // The colour palette, from the 홈 tab.
      click($$('.ribbon__tab').find((t) => t.textContent.trim() === '홈'));
      await settle(4);
      const textColor = () => $$('.colorbtn').find((b) => b.getAttribute('aria-label') === '글자 색');
      // Nothing selected: the control is dead, the way Office greys it out.
      check('선택이 없으면 색 단추가 잠김', textColor()?.disabled === true);
      // The canvas selects on pointerdown, as it does when dragging.
      const block = $('.canvas .block');
      fire(block, 'pointerdown', { button: 0, pointerId: 1 });
      fire(block, 'pointerup', { button: 0, pointerId: 1 });
      await settle(8);
      // Re-queried: selecting a block re-renders the ribbon.
      check('선택하면 색 단추가 열림', textColor()?.disabled === false);
      click(textColor());
      await settle(6);
      const palette = $('.palette');
      check('색 팔레트가 리본 밖에', !!palette && !inRibbon(palette), palette?.parentElement?.className);
      check('팔레트도 body 직속의 팝오버 안에', parentIsBody(palette?.closest('.popover')));
      // Still pickable — being outside the ribbon must not break the click.
      // Where a menu opens is the whole point: it hangs off the control's own
      // box. jsdom measures nothing, so the control is given a box and the
      // panel's placement is read back from it.
      const anchored = await (async () => {
        const control = textColor();
        control.getBoundingClientRect = () => ({
          top: 300, bottom: 326, left: 640, right: 666, width: 26, height: 26, x: 640, y: 300,
        });
        window.dispatchEvent(new window.Event('resize'));
        await settle(6);
        const panel = $('.popover');
        return { top: panel?.style.top, left: panel?.style.left };
      })();
      check('메뉴가 단추 아래에서 열림', anchored.top === '330px', `top = ${anchored.top}`);
      check('메뉴가 단추 왼쪽에 맞춰짐', anchored.left === '640px', `left = ${anchored.left}`);

      // Near the bottom of the window there is no room below, so it flips above;
      // near the right edge it is pulled back so it stays on screen.
      const flipped = await (async () => {
        const control = textColor();
        const height = window.innerHeight || 768;
        const width = window.innerWidth || 1024;
        control.getBoundingClientRect = () => ({
          top: height - 30, bottom: height - 4, left: width - 20, right: width + 6,
          width: 26, height: 26, x: width - 20, y: height - 30,
        });
        window.dispatchEvent(new window.Event('resize'));
        await settle(6);
        const panel = $('.popover');
        return { top: parseFloat(panel?.style.top), left: parseFloat(panel?.style.left) };
      })();
      check('아래 자리가 없으면 위로 뒤집음', flipped.top < (window.innerHeight || 768) - 30,
        `top = ${flipped.top}`);
      check('오른쪽으로 넘치지 않게 당겨짐', flipped.left <= (window.innerWidth || 1024) - 8,
        `left = ${flipped.left}`);

      const target = $$('.palette .swatch')[14];
      const styleOf = () => $('.block.is-selected .block__content')?.getAttribute('style') ?? '';
      const before = styleOf();
      click(target);
      await settle(8);
      check('팝오버 안에서 색을 고를 수 있음', !$('.palette'), '고르면 닫힘');
      check('고른 색이 선택한 블록에 반영됨', styleOf() !== before && /color/.test(styleOf()),
        `${before} → ${styleOf()}`);
    },
  });
  check('메뉴 팝오버에 오류 없음', errors.length === 0, errors.join('\n      '));
}

console.log('\n■ 16:9가 아닌 덱 (4:3)');
{
  const folder = '스모크-표준.aideck';
  const { errors } = await mount(`#/deck/${encodeURIComponent(folder)}`, {
    interact: async ({ settle, $, $$, click, selectValue, setValue }) => {
      const canvas = $('.canvas');
      check('캔버스가 파일의 크기로 열림',
        Math.round(parseFloat(canvas.style.width)) === 960 && Math.round(parseFloat(canvas.style.height)) === 720,
        `${canvas?.style.width} x ${canvas?.style.height}`);

      // A 4:3 deck stretched into a 16:9 thumbnail is the first thing that looks
      // wrong about it.
      const thumb = $('.sorter-item__thumb');
      check('썸네일도 파일의 비율', thumb?.style.aspectRatio === '960 / 720', thumb?.style.aspectRatio);

      // The corner shape is at the corner of *this* canvas.
      const shape = $$('.block').find((b) => b.textContent.includes('우측 하단'));
      const left = parseFloat(shape?.style.left);
      check('도형이 이 캔버스의 우측 하단에', left > 600 && left < 960, shape?.style.left);

      const design = $$('.ribbon__tab').find((t) => t.textContent.trim() === '디자인');
      click(design);
      await settle(4);
      const select = $$('select').find((el) => el.getAttribute('title') === '슬라이드 크기');
      check('크기 목록이 이 파일의 크기를 가리킴', select?.value === '960x720', select?.value);
      const labels = [...select.options].map((o) => o.textContent);
      check('PowerPoint의 크기들을 제공',
        labels.some((l) => l.includes('표준 (4:3)')) && labels.some((l) => l.includes('A4')),
        labels.join(' · '));
      check('비율을 함께 보여줌', $('.ribbon__body').textContent.includes('4:3'),
        sample($('.ribbon__body').textContent));

      // Any size at all, typed in — and the blocks stay on the canvas.
      const width = $$('input[type="number"]').find((el) => el.getAttribute('title') === '너비 (px)');
      check('너비를 직접 입력할 수 있음', !!width);
      setValue(width, '700');
      await settle(8);
      const resized = $('.canvas');
      check('입력한 크기로 캔버스가 바뀜', Math.round(parseFloat(resized.style.width)) === 700,
        resized?.style.width);
      const moved = $$('.block').find((b) => b.textContent.includes('우측 하단'));
      const box = { x: parseFloat(moved.style.left), w: parseFloat(moved.style.width) };
      check('좁아진 캔버스 안으로 도형이 들어옴', box.x + box.w <= 700, JSON.stringify(box));
      const sizeSelect = $$('select').find((el) => el.getAttribute('title') === '슬라이드 크기');
      check('이름 없는 크기는 사용자 지정으로',
        [...sizeSelect.options].some((o) => o.textContent.includes('사용자 지정')),
        [...sizeSelect.options].map((o) => o.textContent).join(' · '));
    },
  });
  check('4:3 덱에 오류 없음', errors.length === 0, errors.join('\n      '));
}

console.log('\n■ Deck 겹친 요소 · 캔버스 밖 배치 · 편집 글꼴 · 패널 토글');
{
  const got = {};
  const { errors } = await mount(`#/deck/${encodeURIComponent('스모크-표준.aideck')}`, {
    async interact({ window, settle, $, $$, fire, click, dblclick }) {
      // Canvas coords → screen px, using the canvas's live scale transform.
      const canvas = $('.canvas');
      const m = canvas?.style.transform.match(/scale\(([\d.]+)\)/);
      const scale = m ? parseFloat(m[1]) : 1;
      const px = (x) => Math.round(x * scale);
      const point = { x: px(340), y: px(340) };

      const front = () => $$('.canvas .block').find((b) => b.textContent.includes('앞 블록'));
      const back = () => $$('.canvas .block').find((b) => b.textContent.includes('뒤 블록'));

      // Alt+click — a click that never moves steps down the stack.
      fire(front(), 'pointerdown', { button: 0, pointerId: 7, altKey: true, clientX: point.x, clientY: point.y });
      await settle(4);
      fire(front(), 'pointerup', { button: 0, pointerId: 7 });
      await settle(4);
      got.altSelectedBack = $('.block.is-selected')?.textContent?.includes('뒤 블록');

      // Right-click on the top block lists the object underneath.
      fire(front(), 'contextmenu', { button: 2, clientX: point.x, clientY: point.y });
      await settle(3);
      const menuText = $('.ctxmenu')?.textContent ?? '';
      got.menuBehind = menuText.includes('뒤에 있는 요소') && menuText.includes('뒤 블록');
      fire(window.document.body, 'mousedown', { button: 0 });
      await settle(3);

      // Dragging the front block far right of the canvas edge is allowed now.
      fire(front(), 'pointerdown', { button: 0, pointerId: 8, clientX: point.x, clientY: point.y });
      await settle(4);
      fire(front(), 'pointermove', { button: 0, pointerId: 8, clientX: point.x + 600, clientY: point.y + 60 });
      await settle(4);
      fire(front(), 'pointerup', { button: 0, pointerId: 8 });
      await settle(5);
      const moved = $$('.canvas .block').find((b) => b.textContent.includes('앞 블록'));
      const box = moved ? { x: parseFloat(moved.style.left), w: parseFloat(moved.style.width) } : null;
      got.beyondCanvas = !!box && box.x + box.w > 960;

      // The edit box wears the block's type style, not a fixed small mono font.
      dblclick(back());
      await settle(4);
      const ed = $('.block__editor');
      got.editorFont = ed?.style?.fontSize;
      got.editorWeight = ed?.style?.fontWeight;
      got.editorAlign = ed?.style?.textAlign;

      // The right panel closes from its own ✕ and reopens from the titlebar.
      const closeBtn = $('.panel__close');
      got.hasCloseBtn = !!closeBtn;
      if (closeBtn) {
        click(closeBtn);
        await settle(4);
        got.panelClosed = !$('.panel--right');
      }
      const toggle = $$('.tbtn').find((b) => b.textContent.includes('저장 포맷'));
      if (toggle) {
        click(toggle);
        await settle(4);
        got.panelReopened = !!$('.panel--right');
      }
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('Alt+클릭이 겹친 아래 요소를 선택', got.altSelectedBack === true);
  check('우클릭 메뉴에 뒤에 있는 요소가 나옴', got.menuBehind === true);
  check('블록이 캔버스 밖으로 이동할 수 있음', got.beyondCanvas === true);
  check('편집 상자가 블록 글꼴 크기를 따름', got.editorFont === '40px', `fontSize = ${got.editorFont}`);
  check('편집 상자가 굵기를 따름', got.editorWeight === '700', `fontWeight = ${got.editorWeight}`);
  check('편집 상자가 정렬을 따름', got.editorAlign === 'left', `textAlign = ${got.editorAlign}`);
  check('패널에 닫기 단추가 있음', got.hasCloseBtn === true);
  check('✕로 패널이 닫힘', got.panelClosed === true);
  check('제목줄 단추로 패널이 다시 열림', got.panelReopened === true);
}

console.log('\n■ Deck Ctrl+휠 확대 · 축소');
{
  const got = {};
  const { errors } = await mount(`#/deck/${encodeURIComponent(byType.deck)}`, {
    async interact({ window, settle, $, fire }) {
      const stage = $('.stage');
      const wheel = (init) => {
        const Ctor = window.WheelEvent ?? window.MouseEvent;
        stage.dispatchEvent(new Ctor('wheel', { bubbles: true, cancelable: true, ...init }));
      };
      const scaleOf = () => {
        const m = $('.canvas')?.style.transform.match(/scale\(([\d.]+)\)/);
        return m ? parseFloat(m[1]) : null;
      };
      got.start = scaleOf();
      got.modeBefore = $('.statusbar button')?.textContent?.trim();

      // Without Ctrl the wheel must leave the view alone.
      wheel({ deltaY: -120 });
      await settle(4);
      got.afterPlain = scaleOf();

      // Ctrl+wheel zooms in and leaves "fit" mode.
      wheel({ deltaY: -120, ctrlKey: true });
      await settle(4);
      got.zoomedIn = scaleOf();
      got.modeAfter = $('.statusbar button')?.textContent?.trim();

      // And back out.
      wheel({ deltaY: 120, ctrlKey: true });
      await settle(4);
      got.zoomedOut = scaleOf();
    },
  });

  check('상호작용 중 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('Ctrl 없이 휠은 확대하지 않음', got.afterPlain === got.start, `scale ${got.start} → ${got.afterPlain}`);
  check('Ctrl+휠 위로 확대함', typeof got.zoomedIn === 'number' && got.zoomedIn > got.start,
    `scale ${got.start} → ${got.zoomedIn}`);
  check('수동 확대 모드로 전환', got.modeAfter === '직접 지정', `mode = ${got.modeAfter}`);
  check('Ctrl+휠 아래로 축소함', got.zoomedOut < got.zoomedIn, `${got.zoomedIn} → ${got.zoomedOut}`);
}

console.log('\n■ 이미지 삽입 (Doc)');
{
  const { errors } = await mount(`#/doc/${encodeURIComponent(byType.doc)}`, {
    interact: async ({ settle, $, $$, click, setValue }) => {
      const insert = $$('.ribbon__tab').find((t) => t.textContent.trim() === '삽입');
      click(insert);
      await settle(4);
      const button = $$('button').find((b) => b.textContent.includes('이미지'));
      check('이미지 버튼이 있음', !!button);
      click(button);
      await settle(6);

      const dialog = $('.dialog');
      check('이미지 대화상자가 열림', !!dialog);
      const url = $$('.dialog input').find((el) => el.type !== 'file');
      check('주소를 넣을 칸이 있음', !!url);
      // A data URL keeps the check offline; the point is that a block appears
      // and renders as an image.
      setValue(url, 'data:image/gif;base64,R0lGODlhAQABAAAAACw=');
      const alt = $$('.dialog input').filter((el) => el.type !== 'file')[1];
      if (alt) setValue(alt, '도표');
      const confirm = $$('.dialog__foot button').find((b) => b.textContent.trim() === '삽입');
      check('삽입 버튼이 있음', !!confirm);
      click(confirm);
      await settle(10);

      const img = $('.page img');
      check('페이지에 이미지가 그려짐', !!img, 'page img');
      check('대체 텍스트가 붙음', img?.getAttribute('alt')?.length > 0, img?.getAttribute('alt'));
    },
  });
  check('이미지 삽입에 오류 없음', errors.length === 0, errors.join('\n      '));
}

console.log('\n■ 머리글 · 바닥글 · 페이지 번호 (Doc)');
{
  const { errors } = await mount(`#/doc/${encodeURIComponent(byType.doc)}`, {
    interact: async ({ settle, $, $$, click, setValue }) => {
      const insert = $$('.ribbon__tab').find((t) => t.textContent.trim() === '삽입');
      click(insert);
      await settle(4);

      const groups = $$('.ribbon__body .rgroup__label').map((l) => l.textContent.trim());
      check('삽입 탭에 머리글 및 바닥글 그룹이 있음', groups.includes('머리글 및 바닥글'), groups.join(', '));

      // Office's one-click page number: the footer's centre slot.
      const pageNumber = $$('button').find((b) => b.textContent.includes('페이지 번호'));
      check('페이지 번호 버튼이 있음', !!pageNumber);
      click(pageNumber);
      await settle(8);
      const footer = $('.running--footer');
      check('바닥글이 페이지에 그려짐', !!footer, 'running--footer');
      check('페이지 번호가 숫자로 치환됨', footer?.textContent.includes('1'), footer?.textContent);
      check('토큰이 그대로 남지 않음', !footer?.textContent.includes('{PAGE}'), footer?.textContent);

      // And the header dialog writes all three slots.
      const header = $$('button').find((b) => b.textContent.includes('머리글'));
      click(header);
      await settle(6);
      const inputs = $$('.dialog input');
      check('머리글 대화상자에 세 칸이 있음', inputs.length >= 3, String(inputs.length));
      setValue(inputs[1], 'AI Studio 제안서');
      const confirm = $$('.dialog button').find((b) => b.textContent.trim() === '확인');
      click(confirm);
      await settle(8);
      const band = $('.running--header');
      check('머리글이 가운데 칸에 그려짐',
        band?.querySelectorAll('.running__slot')[1]?.textContent === 'AI Studio 제안서',
        band?.textContent);
    },
  });
  check('머리글·바닥글에 오류 없음', errors.length === 0, errors.join('\n      '));
}

console.log('\n■ 셀 글꼴 크기 (Grid)');
{
  const { errors } = await mount(`#/grid/${encodeURIComponent(byType.grid)}`, {
    interact: async ({ settle, $, $$, setValue, cellText }) => {
      const box = $$('input[type="number"]').find((el) => el.getAttribute('title') === '글꼴 크기');
      check('글꼴 크기 입력이 있음', !!box, '홈 탭 글꼴 그룹');
      check('기본값은 시트가 그리는 크기', box && Number(box.value) === 15, box?.value);

      setValue(box, '24');
      await settle(6);
      // A1 is the selected cell on load.
      const cell = $('.sheet tbody tr td .cell');
      check('셀 글꼴 크기가 상대 크기로 적용됨', cell?.style.fontSize?.endsWith('em'), cell?.style.fontSize);
      check('셀 내용은 그대로', cellText(1, 0).length > 0, cellText(1, 0));
    },
  });
  check('셀 글꼴 크기 변경에 오류 없음', errors.length === 0, errors.join('\n      '));
}

console.log('\n■ 문서 글꼴 (한 글꼴로 그리기)');
{
  const css = fs.readFileSync(path.join(ROOT, 'apps/web/src/styles.css'), 'utf8');
  check('번들된 글꼴을 @font-face로 선언', /@font-face[\s\S]*?PretendardVariable\.woff2/.test(css));
  check('네트워크에서 글꼴을 받아오지 않음', !/@import\s+url\(https|fonts\.googleapis/.test(css));
  check('문서 표면에 --doc-font를 씀', /--doc-font:/.test(css) && /\.canvas \{[\s\S]*?font-family: var\(--doc-font\)/.test(css));
  const font = path.join(ROOT, 'apps/web/src/fonts/PretendardVariable.woff2');
  check('글꼴 파일이 저장소에 있음', fs.existsSync(font) && fs.statSync(font).size > 500000);
  check('글꼴 라이선스를 함께 담음', fs.existsSync(path.join(ROOT, 'apps/web/src/fonts/OFL.txt')));
}

console.log('\n■ 기존 파일 열기 (런처)');
{
  const { html, text, errors } = await mount('');
  check('런처에 런타임 오류 없음', errors.length === 0, errors.join('\n      '));
  check('기존 파일 열기 카드가 있음', text.includes('기존 파일 열기'));
  check('지원 형식을 밝힘', text.includes('.pptx') && text.includes('.xlsx'));
  check('파일 선택 입력이 있음', /type="file"/.test(html));
}

console.log('\n■ 가져오기 보고 (무엇이 바뀌었는지)');
{
  const { errors } = await mount('', {
    interact: async ({ settle, $, $$, fire, window }) => {
      const input = $('input[type="file"]');
      check('파일 입력이 있음', !!input);
      // A File the launcher will hand to the import API, whose stub answers with
      // the conversion notes.
      const file = new window.File([new Uint8Array([1, 2, 3])], '분기보고.pptx', {
        type: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
      });
      Object.defineProperty(input, 'files', { value: [file], configurable: true });
      fire(input, 'change');
      await settle(30);

      const dialog = $('.dialog');
      check('바뀐 것을 알리는 대화상자가 열림', !!dialog, dialog?.textContent?.slice(0, 80));
      const notes = $$('.importnotes li').map((li) => li.textContent);
      check('토스트가 아니라 목록으로 보여줌', notes.length === 3, notes.join(' | '));
      check('글꼴 치환을 알려줌', notes.some((n) => n.includes('Pretendard')));
      check('SmartArt 변환을 알려줌', notes.some((n) => n.includes('SmartArt')));
      const open = $$('.dialog__foot button').find((b) => b.textContent.trim() === '열기');
      check('열기 버튼이 있음', !!open);

      /*
       * And pressing it actually opens the imported document.
       *
       * Checking only that the button exists is what let a swapped pair of
       * arguments through: the hash came out as `#/<folder>/<type>`, no editor
       * matched it, and the very first thing a new user does — open the .pptx
       * they already have — put them back on the start screen with no error.
       */
      if (open) {
        open.click();
        await settle(30);
      }
      check(
        '열기가 가져온 문서를 실제로 엽니다',
        !!$('.titlebar') && !$('.launcher'),
        `hash = ${window.location.hash}`
      );
      check(
        '주소가 #/<종류>/<폴더> 형태',
        /^#\/(deck|doc|grid)\//.test(window.location.hash),
        `hash = ${window.location.hash}`
      );
    },
  });
  check('가져오기 보고에 오류 없음', errors.length === 0, errors.join('\n      '));
}

console.log(`\n${failures === 0 ? '통과' : '실패'}: ${checks - failures}/${checks} 검사 성공`);
process.exit(failures === 0 ? 0 : 1);

function sample(text) {
  return text.replace(/\s+/g, ' ').slice(0, 300);
}
