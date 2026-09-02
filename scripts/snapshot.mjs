/**
 * Render the real app against real Office files and write a single page of
 * snapshots.
 *
 * Every panel in the output is the app's own DOM, captured after React mounted
 * it, styled by the app's own stylesheet — so what it shows is what the editors
 * draw, not a mock-up of them. Nothing is interactive: the wasm core and the
 * event handlers stay behind in Node.
 *
 *   node scripts/snapshot.mjs <office-files-dir> <out.html>
 *
 * Needs `jsdom` and `esbuild` (see AI_STUDIO_TOOLS), and a running server:
 *   AI_STUDIO_API=http://localhost:5199 node scripts/snapshot.mjs fixtures/ out.html
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..');
const TOOLS = process.env.AI_STUDIO_TOOLS ?? process.env.SCRATCHPAD;
const API = process.env.AI_STUDIO_API ?? 'http://localhost:5199';

if (!TOOLS) {
  console.error('AI_STUDIO_TOOLS=<jsdom과 esbuild가 있는 폴더>를 지정하세요');
  process.exit(2);
}
const require = createRequire(pathToFileURL(path.join(TOOLS, 'package.json')));
const { JSDOM } = require('jsdom');
const esbuild = require('esbuild');

const [fixturesDir, outPath = path.join(ROOT, 'snapshot.html')] = process.argv.slice(2);
if (!fixturesDir) {
  console.error('사용법: node scripts/snapshot.mjs <office-files-dir> [out.html]');
  process.exit(2);
}

/* ------------------------------------------------------------------ import */

/** Open every Office file in the folder through the real API. */
async function importAll(dir) {
  const files = fs
    .readdirSync(dir)
    .filter((name) => /\.(pptx|docx|xlsx)$/i.test(name))
    .sort();
  const out = [];
  for (const name of files) {
    const data = fs.readFileSync(path.join(dir, name)).toString('base64');
    const res = await fetch(`${API}/api/import`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name, data }),
    });
    if (!res.ok) {
      console.error(`  ${name}: ${res.status} ${await res.text()}`);
      continue;
    }
    const result = await res.json();
    out.push({ source: name, ...result });
    console.log(`  ${name} → ${result.folder} (경고 ${result.warnings?.length ?? 0}개)`);
  }
  return out;
}

/* ------------------------------------------------------------------ bundle */

const wasmPath = path.join(ROOT, 'apps/web/src/core/pkg/ai_studio_wasm_bg.wasm');
const wasmBase64 = fs.readFileSync(wasmPath).toString('base64');

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
    sourcefile: 'snapshot-entry.jsx',
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
  loader: { '.css': 'text', '.wasm': 'binary', '.woff2': 'dataurl' },
  external: ['@tauri-apps/api/core'],
  logOverride: { 'empty-import-meta': 'silent', 'ignored-bare-import': 'silent' },
  absWorkingDir: TOOLS,
  nodePaths: [path.join(ROOT, 'node_modules')],
});
const code = bundle.outputFiles[0].text;

/* ------------------------------------------------------------------- mount */

/**
 * Mount the app at a route and return the markup it produced.
 *
 * `act` runs before the capture, for the panels that need a ribbon tab open or a
 * block selected — the same driver style the smoke test uses.
 */
async function capture(hash, act) {
  const dom = new JSDOM(
    `<!doctype html><html><head></head><body><div id="root"></div></body></html>`,
    { url: `http://localhost/${hash}`, pretendToBeVisual: true, runScripts: 'outside-only' }
  );
  const { window } = dom;

  window.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
  window.requestAnimationFrame = (cb) => setTimeout(() => cb(Date.now()), 0);
  window.cancelAnimationFrame = (id) => clearTimeout(id);
  window.scrollTo = () => {};
  // jsdom has no layout, so the scrolling helpers the editors call are absent.
  window.HTMLElement.prototype.scrollIntoView = () => {};
  window.Element.prototype.scrollTo = () => {};
  window.print = () => {};
  Object.defineProperty(window.navigator, 'clipboard', {
    value: { writeText: async () => {} },
    configurable: true,
  });
  // The app asks `/api/...`; the real server answers. Node's own `fetch` is
  // captured first: the stub is installed on globalThis too, so calling `fetch`
  // by name inside it would call the stub.
  const upstream = globalThis.fetch;
  window.fetch = (url, options) => upstream(`${API}${String(url)}`, options);

  const restore = {};
  for (const key of [
    'window', 'document', 'navigator', 'fetch', 'HTMLElement', 'Element', 'Node',
    'ResizeObserver', 'matchMedia', 'getComputedStyle', 'requestAnimationFrame',
    'cancelAnimationFrame', 'FileReader', 'File', 'Blob', 'Event', 'MouseEvent',
    'KeyboardEvent', 'PointerEvent', 'HTMLSelectElement', 'HTMLInputElement',
    'HTMLTextAreaElement',
  ]) {
    restore[key] = global[key];
    global[key] = key === 'getComputedStyle' ? window.getComputedStyle.bind(window) : window[key];
  }

  const errors = [];
  const realError = console.error;
  console.error = (...args) => {
    const text = args.map(String).join(' ');
    if (!/not wrapped in act|useLayoutEffect does nothing/.test(text)) errors.push(text.slice(0, 300));
  };

  let html = '';
  try {
    window.eval(code);
    await window.__aiStudioReady;
    const settle = async (rounds = 30) => {
      for (let i = 0; i < rounds; i++) await new Promise((r) => setTimeout(r, 12));
    };
    await settle();
    if (act) {
      await act({
        window,
        settle,
        $: (sel) => window.document.querySelector(sel),
        $$: (sel) => [...window.document.querySelectorAll(sel)],
        click: (el) => {
          for (const type of ['mousedown', 'mouseup', 'click']) {
            el?.dispatchEvent(new window.MouseEvent(type, { bubbles: true, cancelable: true, button: 0 }));
          }
        },
        /** The canvas selects on pointerdown, as it does when dragging. */
        point: (el) => {
          const Pointer = window.PointerEvent ?? window.MouseEvent;
          for (const type of ['pointerdown', 'pointerup']) {
            el?.dispatchEvent(new Pointer(type, { bubbles: true, cancelable: true, button: 0, pointerId: 1 }));
          }
        },
      });
      await settle(10);
    }
    // An open menu is rendered into `document.body` and positioned against the
    // box the browser measured for its control. jsdom measures nothing, so for
    // the snapshot the panel is moved back beside its control and anchored with
    // CSS instead — which puts it where the running app puts it, under the
    // button that opened it.
    const document_ = window.document;
    const panel = document_.querySelector('.popover');
    const trigger = document_.querySelector('[aria-expanded="true"]');
    if (panel && trigger?.parentElement) {
      panel.className = 'popover popover--anchored';
      panel.removeAttribute('style');
      // Wrapped around the control itself, so the panel hangs from that button
      // and not from whichever one happens to be first in the group.
      const anchor = document_.createElement('span');
      anchor.className = 'sn-anchor';
      trigger.parentElement.insertBefore(anchor, trigger);
      anchor.appendChild(trigger);
      anchor.appendChild(panel);
    }
    html = document_.getElementById('root').innerHTML;
    if (process.env.SNAPSHOT_DEBUG) {
      console.log('DEBUG body:', JSON.stringify(window.document.body.innerHTML.slice(0, 300)));
    }
  } finally {
    console.error = realError;
    for (const [key, value] of Object.entries(restore)) global[key] = value;
    window.close();
  }
  if (errors.length) console.error(`  ⚠ ${hash}: ${errors[0]}`);
  return html;
}

/** Click the nth item of a list — a slide in the sorter, a section in the pane. */
const pick = (selector, index) => async ({ $$, click, settle }) => {
  const items = $$(selector);
  if (items[index]) {
    click(items[index]);
    await settle(8);
  }
};

/** Open a ribbon tab by name. */
const openTab = (name) => async ({ $$, click, settle }) => {
  const tab = $$('.ribbon__tab').find((t) => t.textContent.trim() === name);
  if (tab) {
    click(tab);
    await settle(6);
  }
};

/* --------------------------------------------------------------- rewriting */

/**
 * Inline every asset the markup points at.
 *
 * A snapshot is one file: an `/api/assets` URL would be a broken image
 * everywhere but the machine that made it.
 */
async function inlineAssets(html) {
  const urls = [...new Set([...html.matchAll(/src="(\/api\/[^"]+)"/g)].map((m) => m[1]))];
  let out = html;
  for (const url of urls) {
    try {
      const res = await fetch(`${API}${url.replace(/&amp;/g, '&')}`);
      if (!res.ok) continue;
      const type = res.headers.get('content-type') ?? 'image/png';
      const body = Buffer.from(await res.arrayBuffer()).toString('base64');
      out = out.split(url).join(`data:${type};base64,${body}`);
    } catch {
      /* leave the URL; the panel will show a broken image rather than fail */
    }
  }
  return out;
}

/**
 * A document page grows to fit instead of clipping.
 *
 * Pagination measures rendered block heights, and nothing has a height in jsdom,
 * so every block lands on page one. Letting that page grow shows all of the
 * content; a real browser paginates it.
 */
const growPages = (html) => html.replace(/height:\s*(\d+(?:\.\d+)?)px;\s*max-height/g, 'min-height: $1px; max-height');

/* ------------------------------------------------------------------- write */

const projects = await importAll(fixturesDir);
if (projects.length === 0) {
  console.error('가져온 파일이 없습니다 — 서버가 켜져 있는지 확인하세요');
  process.exit(1);
}

const panels = [];
const byType = (type) => projects.find((p) => p.project.type === type);

for (const { source, folder, project, warnings } of projects) {
  const type = project.type;
  const route = `#/${type}/${encodeURIComponent(folder)}`;
  const label = { deck: '슬라이드', doc: '문서', grid: '시트' }[type] ?? type;
  // A view per thing worth looking at: the ribbon tabs that changed, and the
  // slides and sections that carry a conversion.
  const views =
    type === 'deck'
      ? [
          ['첫 슬라이드', null],
          ['둘째 슬라이드', pick('.sorter-item', 1)],
          ['표 슬라이드', pick('.sorter-item', 3)],
          ['차트 슬라이드', pick('.sorter-item', 4)],
          ['디자인 탭', openTab('디자인')],
          // The colour control, open — the palette is most of what "does this
          // feel like Office" comes down to.
          [
            '색 팔레트',
            async (driver) => {
              await openTab('홈')(driver);
              // The colour control acts on the selection, so a block is selected first
              // — the same order a person works in.
              driver.point(driver.$('.canvas .block'));
              await driver.settle(8);
              const swatch = driver
                .$$('.colorbtn')
                .find((b) => b.getAttribute('aria-label') === '글자 색');
              if (swatch) {
                driver.click(swatch);
                await driver.settle(8);
              }
            },
          ],
        ]
      : type === 'doc'
        ? [
            ['첫 섹션', null],
            ['두 단 섹션', pick('.panel--left .outline-item', 1)],
            ['가로 섹션', pick('.panel--left .outline-item', 2)],
            ['레이아웃 탭', openTab('레이아웃')],
          ]
        : [
            ['첫 시트', null],
            ['요약 시트', pick('.sheettab', 1)],
            ['수식 탭', openTab('수식')],
          ];

  for (const [tab, act] of views) {
    const html = growPages(await inlineAssets(await capture(route, act)));
    panels.push({
      id: `${folder}-${tab}`.replace(/[^\w가-힣-]/g, ''),
      source,
      title: `${source} → ${label}`,
      tab,
      warnings: warnings ?? [],
      html,
    });
    console.log(`  캡처: ${source} · ${tab} (${(html.length / 1024).toFixed(0)}KB)`);
  }
}

const launcher = await capture('', null);
panels.unshift({
  id: 'launcher',
  source: '',
  title: '시작 화면',
  tab: '시작 화면',
  warnings: [],
  html: launcher,
});

const css = fs.readFileSync(path.join(ROOT, 'apps/web/src/styles.css'), 'utf8');
const font = fs.readFileSync(path.join(ROOT, 'apps/web/src/fonts/PretendardVariable.woff2'));
const fontUrl = `data:font/woff2;base64,${font.toString('base64')}`;

/**
 * What each panel is worth looking at for.
 *
 * Keyed by `source|view`. Written by hand because "what changed here" is a
 * judgement, not something the markup can say about itself.
 */
const LOOK_FOR = {
  '|시작 화면': ['기존 파일을 여는 카드와, 열 수 있는 형식 — 창에 끌어다 놓아도 열립니다'],
  'deck.pptx|첫 슬라이드': [
    '제목 59px · 부제 43px — 파일이 아니라 **마스터**가 정한 크기 (44pt / 32pt)',
    '자리틀에서 상속한 좌표 — 슬라이드에는 좌표가 없습니다',
  ],
  'deck.pptx|표 슬라이드': ['열 너비 · 병합(B1:C1) · 오른쪽 정렬이 살아 있는 표'],
  'deck.pptx|차트 슬라이드': ['네이티브 차트의 숫자로 다시 그린 SVG — 그림이 아닙니다'],
  'deck.pptx|디자인 탭': ['슬라이드 크기: PowerPoint와 같은 와이드스크린 / 표준'],
  'themed.pptx|첫 슬라이드': [
    '레이아웃의 띠(#1f3864)와 배경(#f5f7fb) — 슬라이드에는 없고 레이아웃에 있는 것',
    '바닥글 "AI Studio · 대외비"와 오른쪽 아래 **슬라이드 번호** — 마스터의 자리틀에서',
  ],
  'themed.pptx|표 슬라이드': ['같은 디자인 층이 모든 슬라이드에 — 번호는 그 슬라이드의 번호'],
  'themed.pptx|차트 슬라이드': ['디자인 층 위에 놓인 차트'],
  'themed.pptx|디자인 탭': ['가져온 덱의 크기가 목록에 그대로'],
  'themed.pptx|둘째 슬라이드': ['같은 디자인 층 위의 다음 슬라이드 — 번호도 2'],
  'themed.pptx|색 팔레트': [
    'Office와 같은 팔레트 — **테마 색 10열 × 6행**(밝게 3단계·어둡게 2단계), **표준 색 10개**',
    '`자동`으로 색을 지우고, `다른 색…`으로 아무 색이나 — 리본에 고정 견본 6개가 아닙니다',
    '팔레트는 **자기를 연 단추 아래**에서 열리고, 리본 경계를 넘어가도 잘리지 않습니다',
  ],
  'deck.pptx|둘째 슬라이드': ['목록 단계마다 크기가 줄어듭니다 — PowerPoint의 32/28pt'],
  'deck.pptx|색 팔레트': ['테마 색의 첫 강조색은 이 문서의 강조색입니다'],
  'standard-4x3.pptx|첫 슬라이드': [
    '**960 × 720px** — 파일이 4:3이면 캔버스도 4:3입니다',
    '왼쪽 썸네일도 4:3 — 16:9로 늘어나지 않습니다',
  ],
  'standard-4x3.pptx|둘째 슬라이드': [
    '오른쪽 아래 도형이 **이 캔버스의** 우측 하단에 — 16:9로 열면 한가운데에 떠 있게 됩니다',
  ],
  'standard-4x3.pptx|디자인 탭': [
    '크기 목록이 **표준 (4:3)**을 가리키고, 옆에 비율 `4:3`이 적혀 있습니다',
    '너비·높이를 직접 넣으면 어떤 크기로든 — 이름 없는 크기는 `사용자 지정`이 됩니다',
  ],
  'portrait-a4.pptx|첫 슬라이드': [
    '**745 × 1056px 세로 덱** — 창에 맞춤이 높이를 기준으로 잡습니다',
    '레이아웃이 다른 크기(가로 10인치)로 만들어져 있어서, 넘치는 자리틀은 캔버스 안으로 들어옵니다',
  ],
  'doc.docx|첫 섹션': ['제목 단계 · 목록 · 인용 · 용지와 여백', '제목 크기는 Word의 스타일에서 (16pt → 21px)'],
  'doc.docx|두 단 섹션': ['이 파일에는 한 단만 있습니다'],
  'doc.docx|가로 섹션': ['이 파일에는 세로 페이지만 있습니다'],
  'doc.docx|레이아웃 탭': ['Word 순서: 여백 → 용지 방향 → 용지 크기 → 단'],
  'paper.docx|첫 섹션': [
    '머리글 가운데 정렬, 바닥글 왼쪽 "기밀" + 가운데 **페이지 번호**',
    '문장 끝의 **¹**와, 구분선 아래 12px 각주 — Word가 페이지 아래에 두는 그 배치',
  ],
  'paper.docx|두 단 섹션': ['실제 두 단 조판, Word와 같은 48px 간격'],
  'paper.docx|가로 섹션': ['가로 A4 1123×794 — 이름은 A4, 비율은 가로'],
  'paper.docx|레이아웃 탭': ['용지 방향 · 용지 크기 · 단을 여기서 바꿉니다'],
  'excel.xlsx|첫 시트': [
    '수식이 수식으로 (`E2` = `=SUM(B2:D2)`), 표시 형식은 ₩#,##0 과 0.0%',
    '틀 고정과 머리글 스타일',
  ],
  'excel.xlsx|요약 시트': ['이 파일에는 시트가 하나입니다'],
  'excel.xlsx|수식 탭': ['Excel의 함수 범주 7개 — 215개 함수가 들어 있습니다'],
  'report.xlsx|첫 시트': [
    '**조건부 서식이 고정 서식으로** — 증감 열의 음수는 빨강/분홍, 점수 열은 열지도',
    '수식 규칙(`$C2>$B2`)이 목표를 넘긴 부문 이름만 굵게',
    '회계 형식 음수가 `(45)` — `#,##0_);[Red](#,##0)`',
  ],
  'report.xlsx|요약 시트': [
    '**시트를 넘는 수식** — `=SUM(실적!C2:C6)`이 ₩3,638,000,000',
    'XLOOKUP · SUMIFS · STDEV.S · PMT · EOMONTH · NETWORKDAYS의 실제 값',
  ],
  'report.xlsx|수식 탭': ['재무 · 논리 · 텍스트 · 날짜 · 찾기 · 수학 · 통계'],
};

const RUN = [
  ['Rust', '389'],
  ['웹', '60'],
  ['API·디스크', '80'],
  ['화면(jsdom)', '197'],
];

const shell = `
/* ---- The frame around the snapshots. The app's own stylesheet is above; these
   names are prefixed so the two cannot collide. A deliberately single-theme
   dark mount: every panel inside it is a light Office window, and a dark ground
   is what makes them read as objects on a table. ---- */
.sn {
  --sn-ground: #16171b;
  --sn-mount: #20222a;
  --sn-rule: #31343d;
  --sn-ink: #e9ebef;
  --sn-muted: #99a0ac;
  --sn-accent: #5b9cf3;
  --sn-pass: #63be7b;
  --sn-mono: "Cascadia Mono", Consolas, "SF Mono", ui-monospace, monospace;

  background: var(--sn-ground);
  color: var(--sn-ink);
  font-family: var(--font);
  min-height: 100vh;
  padding-bottom: 48px;
}
.sn__head { padding: 28px 28px 0; display: flex; flex-direction: column; gap: 10px; }
.sn__head h1 {
  margin: 0; font-size: 22px; font-weight: 700; letter-spacing: -.01em; text-wrap: balance;
}
.sn__head p { margin: 0; max-width: 68ch; font-size: 13px; line-height: 1.7; color: var(--sn-muted); }
.sn__head strong { color: var(--sn-ink); font-weight: 600; }
.sn__run { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 2px; }
.sn__stat {
  display: flex; align-items: baseline; gap: 6px;
  border: 1px solid var(--sn-rule); border-radius: 6px; padding: 5px 10px;
  font-size: 12px; color: var(--sn-muted); background: var(--sn-mount);
}
.sn__stat b { color: var(--sn-pass); font-family: var(--sn-mono); font-variant-numeric: tabular-nums; font-size: 13px; }

.sn__index { padding: 20px 28px 0; display: flex; flex-direction: column; gap: 10px; }
.sn__group { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.sn__file {
  font-family: var(--sn-mono); font-size: 11px; letter-spacing: .04em; text-transform: uppercase;
  color: var(--sn-muted); min-width: 96px;
}
.sn__index button {
  border: 1px solid var(--sn-rule); background: var(--sn-mount); color: var(--sn-ink);
  border-radius: 999px; padding: 5px 12px; font: inherit; font-size: 12.5px; cursor: pointer;
}
.sn__index button:hover { border-color: var(--sn-accent); }
.sn__index button:focus-visible { outline: 2px solid var(--sn-accent); outline-offset: 2px; }
.sn__index button[aria-pressed="true"] {
  background: var(--sn-accent); border-color: var(--sn-accent); color: #0b1220; font-weight: 600;
}

.sn__panel { padding: 22px 28px 0; }
.sn__caption { display: flex; flex-direction: column; gap: 8px; margin-bottom: 12px; }
.sn__route {
  font-family: var(--sn-mono); font-size: 11.5px; color: var(--sn-muted);
}
.sn__look, .sn__notes { margin: 0; padding-left: 18px; font-size: 13px; line-height: 1.75; }
.sn__look li::marker { color: var(--sn-accent); }
.sn__notes { color: var(--sn-muted); font-size: 12.5px; }
.sn__notes li::marker { color: var(--sn-rule); }
.sn__label {
  font-size: 11px; letter-spacing: .06em; text-transform: uppercase; color: var(--sn-muted);
  font-family: var(--sn-mono);
}
.sn__mount {
  border: 1px solid var(--sn-rule); border-radius: 10px; overflow: hidden;
  background: var(--surface-2); box-shadow: 0 22px 50px rgba(0, 0, 0, .5);
}
/* The app expects the whole viewport; in a panel it gets a fixed box. */
.sn__mount { position: relative; }
.sn__mount .app { height: 820px; }
/* An open menu, hanging from the control that opened it. The running app
   positions it from the control's measured box; a static page has no
   measurements, so it is anchored with CSS to the same place. */
.sn-anchor { position: relative; display: inline-flex; }
.sn__mount .popover--anchored {
  position: absolute; top: calc(100% + 4px); left: 0; max-height: none;
}
/* The ribbon scrolls sideways in the app, which is what used to clip these; in a
   snapshot there is nothing to scroll, so the menu can simply overhang. */
.sn__mount .ribbon__body { overflow: visible; }
.sn__mount .ribbon { position: relative; z-index: 5; }
.sn__mount .doc-scroll, .sn__mount .canvas-scroll { overflow: hidden; }
.sn__foot {
  padding: 26px 28px 0; font-size: 12px; line-height: 1.8; color: var(--sn-muted); max-width: 78ch;
}
.sn__foot code { font-family: var(--sn-mono); background: var(--sn-mount); padding: 1px 5px; border-radius: 3px; }
[hidden] { display: none !important; }
`;

const groups = [];
for (const panel of panels) {
  const key = panel.source || '시작';
  let group = groups.find((g) => g.key === key);
  if (!group) {
    group = { key, items: [] };
    groups.push(group);
  }
  group.items.push(panel);
}

const index = groups
  .map(
    (group) => `  <div class="sn__group"><span class="sn__file">${escapeHtml(group.key)}</span>
${group.items
  .map(
    (panel) =>
      `    <button type="button" data-target="${panel.id}">${escapeHtml(panel.tab || '화면')}</button>`
  )
  .join('\n')}
  </div>`
  )
  .join('\n');

const body = panels
  .map((panel, i) => {
    const look = LOOK_FOR[`${panel.source}|${panel.tab}`] ?? [];
    const bold = (text) =>
      escapeHtml(text)
        .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
        .replace(/`([^`]+)`/g, '<code>$1</code>');
    return `<section class="sn__panel" id="${panel.id}"${i === 0 ? '' : ' hidden'}>
  <div class="sn__caption">
    <span class="sn__route">${escapeHtml(panel.source ? `${panel.source} → ${panel.title.split('→')[1].trim()}` : panel.title)}${
      panel.tab ? ` · ${escapeHtml(panel.tab)}` : ''
    }</span>
    ${look.length ? `<ul class="sn__look">${look.map((l) => `<li>${bold(l)}</li>`).join('')}</ul>` : ''}
    ${
      panel.warnings.length
        ? `<div><span class="sn__label">가져올 때 바꾼 것</span><ul class="sn__notes">${panel.warnings
            .map((w) => `<li>${escapeHtml(w)}</li>`)
            .join('')}</ul></div>`
        : ''
    }
  </div>
  <div class="sn__mount">${panel.html}</div>
</section>`;
  })
  .join('\n');

const page = `<title>AI Studio 화면 확인</title>
<style>
${css.replace(/url\("\.\/fonts\/PretendardVariable\.woff2"\)/, `url("${fontUrl}")`)}
${shell}
</style>
<div class="sn">
  <header class="sn__head">
    <h1>AI Studio — 실제 화면으로 확인하기</h1>
    <p>
      아래 화면은 각 Office 파일을 <strong>실제 가져오기 코드</strong>로 열고 <strong>실제 에디터</strong>를
      띄워 나온 DOM을, 앱의 스타일시트와 번들된 글꼴과 함께 그대로 담은 것입니다. 목업이 아니라 앱이
      그린 화면이고, 그래서 눌러도 아무 일도 일어나지 않습니다 — 계산 코어는 서버에 남아 있습니다.
    </p>
    <div class="sn__run">
      ${RUN.map(([label, count]) => `<span class="sn__stat">${label} <b>${count}</b></span>`).join('\n      ')}
      <span class="sn__stat">clippy · fmt <b>0</b></span>
    </div>
  </header>
  <nav class="sn__index" id="sn-index">
${index}
  </nav>
${body}
  <p class="sn__foot">
    문서 화면은 페이지 나눔 계산 없이 한 장에 담겨 있습니다 — 블록 높이를 재려면 실제 브라우저의
    레이아웃이 필요하고, 이 스냅샷은 그 전 단계에서 떠 온 것입니다. 시트의 <code>#DIV/0!</code> 같은
    값은 원본 파일에 그렇게 들어 있는 것입니다. 다시 만들려면:
    <code>node scripts/snapshot.mjs &lt;office-files&gt; out.html</code>
  </p>
</div>
<script>
  const buttons = [...document.querySelectorAll('#sn-index button')];
  const show = (id) => {
    for (const button of buttons) {
      const on = button.dataset.target === id;
      button.setAttribute('aria-pressed', String(on));
      document.getElementById(button.dataset.target).hidden = !on;
    }
    window.scrollTo({ top: 0, behavior: 'instant' });
  };
  for (const button of buttons) button.addEventListener('click', () => show(button.dataset.target));
  if (buttons[0]) show(buttons[0].dataset.target);
</script>
`;

fs.writeFileSync(outPath, page);
console.log(`\n${outPath}  ${(fs.statSync(outPath).size / 1024 / 1024).toFixed(1)}MB · 패널 ${panels.length}개`);

function escapeHtml(text) {
  return String(text).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
}
