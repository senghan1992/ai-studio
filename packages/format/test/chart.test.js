import test from 'node:test';
import assert from 'node:assert/strict';

import {
  normalizeChartSpec, parseChartBlock, serializeChartBlock,
  renderChartSvg, chartDataFromRange, resolveChartSpec,
  chartToMarkdownTable, describeChart, CHART_TYPES, MAX_SERIES, CHART_PALETTE,
} from '../src/chart.js';

const columnSpec = {
  type: 'column',
  title: '분기별 매출',
  labels: ['1분기', '2분기', '3분기', '4분기'],
  series: [
    { name: '2025', values: [98, 114, 142, 160] },
    { name: '2026', values: [120, 138, 171, 190] },
  ],
};

/* --------------------------------------------------------------------- spec */

test('an unknown chart type falls back to a column chart', () => {
  assert.equal(normalizeChartSpec({ type: 'sunburst' }).type, 'column');
  for (const type of CHART_TYPES) assert.equal(normalizeChartSpec({ type }).type, type);
});

test('normalizing coerces values and drops malformed series', () => {
  const spec = normalizeChartSpec({
    labels: [1, '이', null],
    series: [{ name: 'a', values: ['10', '', 'x', 3] }, { name: 'broken' }, null],
  });
  assert.deepEqual(spec.labels, ['1', '이', '']);
  assert.equal(spec.series.length, 1, 'a series without values is dropped');
  assert.deepEqual(spec.series[0].values, [10, null, null, 3]);
});

test('a ninth series folds into 기타 rather than inventing a hue', () => {
  const spec = normalizeChartSpec({
    labels: ['A', 'B'],
    series: Array.from({ length: 11 }, (_, i) => ({ name: `계열${i + 1}`, values: [i + 1, 1] })),
  });
  assert.equal(spec.series.length, MAX_SERIES);
  assert.equal(spec.series[MAX_SERIES - 1].name, '기타');
  // Slots 8..11 carried 8+9+10+11 = 38 in the first position.
  assert.equal(spec.series[MAX_SERIES - 1].values[0], 38);
  assert.equal(spec.series[MAX_SERIES - 1].values[1], 4);
});

test('value labels default to selective, never every point', () => {
  assert.equal(normalizeChartSpec({}).options.valueLabels, 'max');
  assert.equal(normalizeChartSpec({ options: { valueLabels: 'nonsense' } }).options.valueLabels, 'max');
  assert.equal(normalizeChartSpec({ options: { valueLabels: 'all' } }).options.valueLabels, 'all');
});

test('chart blocks round-trip through markdown', () => {
  const md = serializeChartBlock(columnSpec);
  assert.match(md, /^```chart\n/);
  const back = parseChartBlock(md);
  assert.equal(back.title, '분기별 매출');
  assert.deepEqual(back.series[1].values, [120, 138, 171, 190]);
});

test('a malformed chart block yields null instead of throwing', () => {
  assert.equal(parseChartBlock('```chart\n{ not json\n```'), null);
  assert.equal(parseChartBlock('# 그냥 제목'), null);
});

/* ------------------------------------------------------------ sheet ranges */

const sheet = {
  cells: {
    A1: { v: '지역', t: 's' }, B1: { v: '1분기', t: 's' }, C1: { v: '2분기', t: 's' },
    A2: { v: '서울', t: 's' }, B2: { v: 520, t: 'n' }, C2: { v: 560, t: 'n' },
    A3: { v: '부산', t: 's' }, B3: { v: 180, t: 'n' }, C3: { v: 210, t: 'n' },
  },
};

test('a range becomes labels and series, columns as series', () => {
  const data = chartDataFromRange(sheet, 'A1:C3');
  assert.deepEqual(data.labels, ['서울', '부산']);
  assert.deepEqual(data.series.map((s) => s.name), ['1분기', '2분기']);
  assert.deepEqual(data.series[0].values, [520, 180]);
});

test('rows-as-series flips the orientation', () => {
  const data = chartDataFromRange(sheet, 'A1:C3', { orientation: 'rows' });
  assert.deepEqual(data.labels, ['1분기', '2분기']);
  assert.deepEqual(data.series.map((s) => s.name), ['서울', '부산']);
  assert.deepEqual(data.series[0].values, [520, 560]);
});

test('resolveChartSpec pulls a range-backed spec from the sheet', () => {
  const spec = resolveChartSpec({ type: 'bar', range: 'A1:C3', title: '지역별' }, sheet);
  assert.deepEqual(spec.labels, ['서울', '부산']);
  assert.equal(spec.series.length, 2);
});

test('non-numeric cells in a range become gaps, not zeros', () => {
  const withText = { cells: { ...sheet.cells, B3: { v: '미정', t: 's' } } };
  const data = chartDataFromRange(withText, 'A1:C3');
  assert.deepEqual(data.series[0].values, [520, null]);
});

test('an invalid range yields empty data', () => {
  assert.deepEqual(chartDataFromRange(sheet, 'nonsense'), { labels: [], series: [] });
});

/* -------------------------------------------------------- rendering specs */

const parseSvg = (svg) => ({
  viewBox: svg.match(/viewBox="0 0 (\d+) (\d+)"/)?.slice(1).map(Number) ?? [0, 0],
  texts: [...svg.matchAll(/<text[^>]*x="([-\d.]+)"[^>]*y="([-\d.]+)"[^>]*fill="([^"]+)"[^>]*>([^<]*)<\/text>/g)].map((m) => ({
    x: Number(m[1]), y: Number(m[2]), fill: m[3], text: m[4],
  })),
  lines: [...svg.matchAll(/<line[^>]*>/g)].map((m) => m[0]),
  paths: [...svg.matchAll(/<path[^>]*>/g)].map((m) => m[0]),
  circles: [...svg.matchAll(/<circle[^>]*>/g)].map((m) => m[0]),
  rects: [...svg.matchAll(/<rect[^>]*>/g)].map((m) => m[0]),
});

test('the SVG is well-formed and sized as asked', () => {
  const svg = renderChartSvg(columnSpec, { width: 520, height: 340 });
  assert.match(svg, /^<svg /);
  assert.match(svg, /<\/svg>$/);
  assert.equal((svg.match(/<svg/g) ?? []).length, 1);
  assert.deepEqual(parseSvg(svg).viewBox, [520, 340]);
  assert.match(svg, /role="img"/);
  assert.match(svg, /aria-label="분기별 매출"/);
});

test('no text escapes the viewBox', () => {
  for (const type of CHART_TYPES) {
    const svg = renderChartSvg({ ...columnSpec, type }, { width: 520, height: 340 });
    const { viewBox, texts } = parseSvg(svg);
    for (const t of texts) {
      assert.ok(t.x >= -2 && t.x <= viewBox[0] + 2, `${type}: text "${t.text}" x=${t.x} outside 0..${viewBox[0]}`);
      assert.ok(t.y >= 0 && t.y <= viewBox[1] + 2, `${type}: text "${t.text}" y=${t.y} outside 0..${viewBox[1]}`);
    }
  }
});

test('long CJK category labels are thinned rather than overlapped', () => {
  const many = {
    type: 'column',
    labels: Array.from({ length: 24 }, (_, i) => `아주긴항목이름${i + 1}`),
    series: [{ name: 'v', values: Array.from({ length: 24 }, (_, i) => i + 1) }],
  };
  const svg = renderChartSvg(many, { width: 420, height: 300 });
  const labels = parseSvg(svg).texts.filter((t) => t.text.startsWith('아주긴항목이름'));
  assert.ok(labels.length < 24, `expected thinning, drew ${labels.length} of 24`);
  assert.ok(labels.length >= 2, 'but not all of them removed');
});

test('gridlines and axes are solid hairlines, never dashed', () => {
  const svg = renderChartSvg(columnSpec, { width: 520, height: 340 });
  const { lines } = parseSvg(svg);
  assert.ok(lines.length > 0, 'gridlines are drawn');
  for (const line of lines) {
    assert.ok(!/stroke-dasharray/.test(line), `dashed grid line: ${line}`);
    assert.match(line, /stroke-width="1"/, `non-hairline grid line: ${line}`);
  }
});

test('bars are capped at 24px even when the band is wide', () => {
  // A 900px canvas with two categories gives each a ~430px band; the mark must
  // still cap at 24px and leave the rest as air.
  const svg = renderChartSvg(
    { type: 'column', labels: ['A', 'B'], series: [{ name: 'v', values: [10, 20] }] },
    { width: 900, height: 320 }
  );
  const bars = parseSvg(svg).paths.filter((p) => /d="M[\d.]+ [\d.]+V/.test(p));
  assert.equal(bars.length, 2, `expected 2 bars, found ${bars.length}`);
  for (const bar of bars) {
    const box = columnBox(bar);
    assert.ok(box, `could not measure ${bar}`);
    assert.ok(box.width <= 24.5, `bar ${box.width}px exceeds the 24px cap`);
    assert.ok(box.width >= 6, `bar ${box.width}px is implausibly thin`);
  }
});

test('a positive column is rounded at the top and square at the baseline', () => {
  const svg = renderChartSvg(
    { type: 'column', labels: ['A'], series: [{ name: 'v', values: [10] }] },
    { width: 400, height: 300 }
  );
  const bar = parseSvg(svg).paths.find((p) => /d="M[\d.]+ [\d.]+V/.test(p));
  const box = columnBox(bar);
  // The path starts at the baseline, runs up, and both quadratic curves sit at the
  // top — the data end. Rounding at the baseline instead is the classic sign error.
  const curveYs = [...bar.matchAll(/Q[\d.]+ ([\d.]+)/g)].map((m) => Number(m[1]));
  assert.equal(curveYs.length, 2, 'exactly two rounded corners');
  for (const y of curveYs) {
    assert.ok(Math.abs(y - box.top) < 1, `corner at y=${y} is not at the data end (top=${box.top})`);
    assert.ok(Math.abs(y - box.bottom) > 4, `corner at y=${y} is at the baseline (${box.bottom})`);
  }
});

test('a negative column is rounded at the bottom instead', () => {
  const svg = renderChartSvg(
    { type: 'column', labels: ['A'], series: [{ name: 'v', values: [-10] }] },
    { width: 400, height: 300 }
  );
  const bar = parseSvg(svg).paths.find((p) => /d="M[\d.]+ [\d.]+V/.test(p));
  const box = columnBox(bar);
  const curveYs = [...bar.matchAll(/Q[\d.]+ ([\d.]+)/g)].map((m) => Number(m[1]));
  for (const y of curveYs) {
    assert.ok(Math.abs(y - box.bottom) < 1, `corner at y=${y} should sit at the bottom (${box.bottom})`);
  }
});

test('value-axis tick labels never overlap', () => {
  // Long formatted numbers on a narrow horizontal chart is the worst case.
  const cases = [
    { spec: { type: 'bar', labels: ['아주아주긴이름하나', '둘'], series: [{ name: 'v', values: [1000000, 2] }] }, size: { width: 220, height: 150 } },
    { spec: { type: 'bar', labels: ['A', 'B'], series: [{ name: 'v', values: [123456789, 5] }] }, size: { width: 300, height: 200 } },
    { spec: { type: 'column', labels: ['A', 'B'], series: [{ name: 'v', values: [1, 100000] }] }, size: { width: 240, height: 160 } },
  ];
  for (const { spec, size } of cases) {
    const svg = renderChartSvg(spec, size);
    const horizontal = spec.type === 'bar';
    const ticks = parseSvg(svg).texts.filter((t) => t.fill === '#797775');
    const boxes = ticks
      .map((t) => (horizontal ? [t.x - width11(t.text) / 2, t.x + width11(t.text) / 2] : [t.y - 6, t.y + 6]))
      .sort((a, b) => a[0] - b[0]);
    for (let i = 1; i < boxes.length; i++) {
      assert.ok(
        boxes[i][0] >= boxes[i - 1][1] - 0.5,
        `tick labels overlap at ${JSON.stringify(ticks.map((t) => t.text))} (${boxes[i - 1]} vs ${boxes[i]})`
      );
    }
    assert.ok(ticks.length >= 2, `expected at least 2 tick labels, got ${ticks.length}`);
  }
});

test('line charts use 2px round-capped strokes and markers of at least r4', () => {
  const svg = renderChartSvg({ ...columnSpec, type: 'line' }, { width: 520, height: 340 });
  const strokes = parseSvg(svg).paths.filter((p) => /stroke-width/.test(p));
  assert.ok(strokes.length >= 2, 'one path per series');
  for (const s of strokes) {
    assert.match(s, /stroke-width="2"/);
    assert.match(s, /stroke-linecap="round"/);
    assert.match(s, /fill="none"/);
  }
  const circles = parseSvg(svg).circles;
  assert.ok(circles.length > 0);
  for (const c of circles) {
    const r = Number(c.match(/r="([\d.]+)"/)[1]);
    assert.ok(r >= 4, `marker r=${r} below the 4px minimum`);
    assert.match(c, /stroke-width="2"/, 'markers carry a 2px surface ring');
  }
});

test('area fills are a wash, not a saturated block', () => {
  const svg = renderChartSvg({ ...columnSpec, type: 'area' }, { width: 520, height: 340 });
  const fills = parseSvg(svg).paths.filter((p) => /fill-opacity/.test(p));
  assert.ok(fills.length >= 1);
  for (const f of fills) {
    const opacity = Number(f.match(/fill-opacity="([\d.]+)"/)[1]);
    assert.ok(opacity <= 0.15, `area opacity ${opacity} is too solid`);
  }
});

test('pie slices are separated by a surface gap, not an outline', () => {
  const svg = renderChartSvg(
    { type: 'pie', labels: ['가', '나', '다'], series: [{ name: 'v', values: [3, 4, 5] }] },
    { width: 420, height: 340, surface: '#ffffff' }
  );
  const slices = parseSvg(svg).paths;
  assert.equal(slices.length, 3);
  for (const slice of slices) {
    assert.match(slice, /stroke="#ffffff"/, 'the separator is the surface colour');
    assert.match(slice, /stroke-width="2"/);
  }
});

test('a legend is present for two series and absent for one', () => {
  const two = renderChartSvg(columnSpec, { width: 520, height: 340 });
  assert.ok(two.includes('2025') && two.includes('2026'), 'both series are named');
  const swatches = parseSvg(two).rects.filter((r) => /rx="2"/.test(r));
  assert.equal(swatches.length, 2, 'one swatch per series');

  const one = renderChartSvg(
    { type: 'column', title: '한 계열', labels: ['A'], series: [{ name: '유일', values: [1] }] },
    { width: 520, height: 340 }
  );
  assert.equal(parseSvg(one).rects.filter((r) => /rx="2"/.test(r)).length, 0, 'a single series needs no legend box');
});

test('legend and axis text wear ink tokens, never a series colour', () => {
  const svg = renderChartSvg(columnSpec, { width: 520, height: 340 });
  const seriesColors = new Set([...CHART_PALETTE.light, ...CHART_PALETTE.dark].map((c) => c.toLowerCase()));
  for (const t of parseSvg(svg).texts) {
    // White in-bar labels are contrast relief, not series identity.
    if (t.fill.toLowerCase() === '#ffffff') continue;
    assert.ok(!seriesColors.has(t.fill.toLowerCase()), `text "${t.text}" wears series colour ${t.fill}`);
  }
});

test('the default label policy labels the extreme, not every point', () => {
  const spec = { type: 'column', labels: ['A', 'B', 'C', 'D'], series: [{ name: 'v', values: [5, 40, 12, 9] }] };
  const svg = renderChartSvg(spec, { width: 520, height: 340 });
  const values = parseSvg(svg).texts.map((t) => t.text);
  assert.ok(values.includes('40'), 'the extreme is labelled');
  assert.ok(!values.includes('12'), 'the rest are not');
});

test('negative values render below the zero line', () => {
  const svg = renderChartSvg(
    { type: 'column', labels: ['A', 'B'], series: [{ name: 'v', values: [-40, 60] }] },
    { width: 520, height: 340 }
  );
  const { texts } = parseSvg(svg);
  assert.ok(texts.some((t) => t.text === '-40' || t.text === '0'), 'the axis spans zero');
  assert.match(svg, /<path d="M/, 'bars still drew');
});

test('dark surfaces get the dark palette and light ink', () => {
  const dark = renderChartSvg(columnSpec, { width: 520, height: 340, surface: '#111827' });
  assert.match(dark, /fill="#111827"/, 'the surface is painted');
  assert.ok(dark.includes(CHART_PALETTE.dark[0]), 'dark-mode series colour is used');
  assert.ok(!dark.includes(`fill="${CHART_PALETTE.light[0]}"`), 'not the light-mode step');
  const titles = parseSvg(dark).texts.filter((t) => t.text === '분기별 매출');
  assert.equal(titles[0].fill, '#ffffff', 'title ink flips for the dark surface');
});

test('a chart with no data renders a message instead of an empty frame', () => {
  const svg = renderChartSvg({ type: 'column', title: '없음', labels: [], series: [] }, { width: 300, height: 200 });
  assert.match(svg, /데이터가 없습니다/);
  assert.match(svg, /<\/svg>$/);
});

test('text content is escaped', () => {
  const svg = renderChartSvg(
    { type: 'column', title: '<script>&"', labels: ['<b>'], series: [{ name: 'x', values: [1] }] },
    { width: 300, height: 220 }
  );
  assert.ok(!svg.includes('<script>'), 'raw markup never reaches the output');
  assert.match(svg, /&lt;script&gt;&amp;&quot;/);
});

test('every chart type renders without throwing, for every data shape', () => {
  const shapes = [
    { labels: ['A'], series: [{ name: 's', values: [1] }] },
    { labels: ['A', 'B'], series: [{ name: 's', values: [0, 0] }] },
    { labels: ['A', 'B'], series: [{ name: 's', values: [null, 5] }] },
    { labels: ['A', 'B'], series: [{ name: 'a', values: [1, 2] }, { name: 'b', values: [3, 4] }] },
    { labels: ['A', 'B'], series: [{ name: 'a', values: [-1, -2] }] },
    { labels: ['A', 'B'], series: [{ name: 'a', values: [1e9, 2e9] }] },
  ];
  for (const type of CHART_TYPES) {
    for (const [i, shape] of shapes.entries()) {
      for (const stacked of [false, true]) {
        const svg = renderChartSvg({ type, ...shape, options: { stacked } }, { width: 400, height: 260 });
        assert.match(svg, /<\/svg>$/, `${type} shape ${i} stacked=${stacked}`);
        assert.ok(!svg.includes('NaN'), `${type} shape ${i} stacked=${stacked} produced NaN`);
        assert.ok(!svg.includes('Infinity'), `${type} shape ${i} produced Infinity`);
        assert.ok(!/="undefined"/.test(svg), `${type} shape ${i} produced undefined`);
      }
    }
  }
});

/* ---------------------------------------------------------- the table view */

test('the table view carries the numbers the chart draws', () => {
  const table = chartToMarkdownTable(columnSpec);
  assert.match(table, /\| 구분 \| 2025 \| 2026 \|/);
  assert.match(table, /\| 1분기 \| 98 \| 120 \|/);
  assert.match(table, /\| 4분기 \| 160 \| 190 \|/);
});

test('the description names the type, series and item count', () => {
  const text = describeChart(columnSpec);
  assert.match(text, /세로 막대/);
  assert.match(text, /계열 2개\(2025, 2026\)/);
  assert.match(text, /항목 4개/);
});

test('a range-backed chart mentions its range', () => {
  assert.match(describeChart({ type: 'bar', range: 'A1:C3' }), /범위 `A1:C3`/);
});

/** Real geometry of a column bar path: M x yBase V yEnd Q… H… Q… V yBase Z */
function columnBox(path) {
  const d = path.match(/d="([^"]+)"/)?.[1];
  if (!d) return null;
  const xs = [...d.matchAll(/[MHQL]\s*([\d.]+)/g)].map((m) => Number(m[1]));
  const ys = [...d.matchAll(/(?:M[\d.]+\s+|V|Q[\d.]+\s+)([\d.]+)/g)].map((m) => Number(m[1]));
  if (!xs.length || !ys.length) return null;
  return {
    width: Math.max(...xs) - Math.min(...xs),
    top: Math.min(...ys),
    bottom: Math.max(...ys),
  };
}

/** Approximate rendered width of an 11px label, matching the renderer's metric. */
function width11(text) {
  let units = 0;
  for (const ch of String(text)) units += /[가-힯一-鿿ᄀ-ᇿ　-〿]/.test(ch) ? 1 : 0.55;
  return units * 11;
}

test('a range that resolves to nothing falls back to the inline data', () => {
  // A single-column range cannot produce a series; the spec's own numbers stand in.
  const spec = resolveChartSpec(
    {
      type: 'pie',
      range: 'A1:A3',
      labels: ['가', '나'],
      series: [{ name: '값', values: [3, 7] }],
    },
    sheet
  );
  assert.deepEqual(spec.labels, ['가', '나']);
  assert.deepEqual(spec.series[0].values, [3, 7]);
});

test('a range with real data still wins over inline data', () => {
  const spec = resolveChartSpec(
    { type: 'bar', range: 'A1:C3', labels: ['버릴 것'], series: [{ name: 'x', values: [1] }] },
    sheet
  );
  assert.deepEqual(spec.labels, ['서울', '부산']);
  assert.equal(spec.series.length, 2);
});

test('a broken range falls back rather than rendering empty', () => {
  const spec = resolveChartSpec(
    { type: 'column', range: 'ZZ999:ZZ1000', labels: ['a'], series: [{ name: 's', values: [5] }] },
    sheet
  );
  assert.deepEqual(spec.series[0].values, [5]);
});
