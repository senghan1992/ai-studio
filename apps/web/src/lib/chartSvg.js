/**
 * Chart drawing: a spec in, an SVG string out. SVG rather than canvas so the
 * same markup works in the editor, in the file inspector and in a print view.
 *
 * This is the one piece of chart handling that is not in the Rust core, and
 * deliberately so — it is view code. Nothing on disk and nothing in `AI.md`
 * depends on it: the digest carries the chart's numbers as a table, and the
 * `.pptx` export writes a native chart part. So the picture is drawn where the
 * picture is shown, and the data stays in one place.
 *
 * Marks follow the data-viz specs: bars capped at 24px with a 4px rounded
 * data-end, 2px lines, hairline solid gridlines one step off the surface, a 2px
 * surface gap between touching fills, and value labels only where they earn
 * their place. A legend is always present for two or more series — identity is
 * never colour alone.
 */
import {
  CHART_PALETTE, applyNumFmt, normalizeChartSpec, resolveChartSpec,
} from '../core/index.js';

const PALETTE = CHART_PALETTE;

const INK = {
  light: { primary: '#0b0b0b', secondary: '#52514e', muted: '#797775', grid: '#e6e5e2', axis: '#c9c8c4' },
  dark: { primary: '#ffffff', secondary: '#c3c2b7', muted: '#9a998f', grid: '#3a3a37', axis: '#4d4c48' },
};

const BAR_MAX = 24;
const GAP = 2;
const LINE_W = 2;
const MARKER_R = 4;

/* ----------------------------------------------------------------- render */

/** Render a chart as a standalone SVG string. */
export function renderChartSvg(spec, { width = 640, height = 380, surface = '#ffffff' } = {}) {
  const chart = normalizeChartSpec(spec);
  const mode = isDark(surface) ? 'dark' : 'light';
  const colors = PALETTE[mode];
  const ink = INK[mode];

  const w = Math.max(200, Math.round(width));
  const h = Math.max(140, Math.round(height));

  const series = chart.series.filter((s) => s.values.some((v) => v !== null));
  if (!series.length || (!chart.labels.length && chart.type !== 'pie' && chart.type !== 'donut')) {
    return emptyChart(w, h, surface, ink, chart.title);
  }

  const parts = [];
  parts.push(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}" role="img" aria-label="${esc(chart.title || '차트')}" font-family="Segoe UI, -apple-system, Malgun Gothic, sans-serif">`
  );
  parts.push(`<rect width="${w}" height="${h}" fill="${surface}"/>`);

  let top = 14;
  if (chart.title) {
    parts.push(
      `<text x="${w / 2}" y="24" text-anchor="middle" font-size="15" font-weight="600" fill="${ink.primary}">${esc(chart.title)}</text>`
    );
    top = 40;
  }

  // A legend is always present for two or more series — identity is never colour alone.
  const showLegend = chart.options.legend && (series.length > 1 || chart.type === 'pie' || chart.type === 'donut');
  const legendH = showLegend ? 24 : 0;
  const plotBottom = h - legendH - 8;

  if (chart.type === 'pie' || chart.type === 'donut') {
    parts.push(...renderPie(chart, series, colors, ink, surface, { w, top, bottom: plotBottom }));
  } else {
    parts.push(...renderCartesian(chart, series, colors, ink, surface, { w, top, bottom: plotBottom }));
  }

  if (showLegend) {
    const names =
      chart.type === 'pie' || chart.type === 'donut'
        ? chart.labels.map((l, i) => ({ name: l || `항목 ${i + 1}`, color: colors[i % colors.length] }))
        : series.map((s, i) => ({ name: s.name || `계열 ${i + 1}`, color: colors[i % colors.length] }));
    parts.push(...renderLegend(names, ink, { w, y: h - 10 }));
  }

  parts.push('</svg>');
  return parts.join('');
}

function emptyChart(w, h, surface, ink, title) {
  return [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}" role="img" aria-label="빈 차트">`,
    `<rect width="${w}" height="${h}" fill="${surface}"/>`,
    title
      ? `<text x="${w / 2}" y="24" text-anchor="middle" font-size="15" font-weight="600" fill="${ink.primary}">${esc(title)}</text>`
      : '',
    `<text x="${w / 2}" y="${h / 2}" text-anchor="middle" font-size="13" fill="${ink.muted}">데이터가 없습니다</text>`,
    '</svg>',
  ].join('');
}

/* ------------------------------------------------------ cartesian charts */

function renderCartesian(chart, series, colors, ink, surface, { w, top, bottom }) {
  const horizontal = chart.type === 'bar';
  const stacked = chart.options.stacked && chart.type !== 'line';
  const count = chart.labels.length || Math.max(...series.map((s) => s.values.length));

  const { min, max, ticks } = scaleFor(series, stacked, chart.type);
  const fmt = (v) => formatValue(v, chart.options.numberFormat);

  // Reserve room for the widest tick label so nothing is clipped.
  const tickWidth = Math.max(...ticks.map((t) => textWidth(fmt(t), 11)));
  // Cap the category gutter as a share of the canvas: a long CJK label on a small
  // chart would otherwise eat the plot and crush the value axis.
  const labelWidth = horizontal
    ? Math.min(Math.max(40, w * 0.34), Math.max(...chart.labels.map((l) => textWidth(l, 11)), 20) + 8)
    : 0;

  const pad = {
    top,
    right: 18,
    bottom: horizontal ? 26 : 30,
    left: (horizontal ? labelWidth : tickWidth + 10) + 10,
  };
  const plot = {
    x: pad.left,
    y: pad.top,
    w: Math.max(40, w - pad.left - pad.right),
    h: Math.max(40, bottom - pad.top - pad.bottom),
  };

  const out = [];
  const valueToPos = (v) =>
    horizontal
      ? plot.x + ((v - min) / (max - min || 1)) * plot.w
      : plot.y + plot.h - ((v - min) / (max - min || 1)) * plot.h;

  // Gridlines: hairline, solid, one step off the surface, drawn behind the marks.
  // Every gridline is drawn, but its label is skipped when labels would collide —
  // a crowded axis is unreadable, and a clipped one is worse.
  const keptTicks = selectTickLabels(ticks, valueToPos, (t) => (horizontal ? textWidth(fmt(t), 11) : 13));

  if (chart.options.grid) {
    ticks.forEach((t, ti) => {
      const p = valueToPos(t);
      out.push(
        horizontal
          ? `<line x1="${r1(p)}" y1="${plot.y}" x2="${r1(p)}" y2="${plot.y + plot.h}" stroke="${ink.grid}" stroke-width="1"/>`
          : `<line x1="${plot.x}" y1="${r1(p)}" x2="${plot.x + plot.w}" y2="${r1(p)}" stroke="${ink.grid}" stroke-width="1"/>`
      );
      if (!keptTicks.has(ti)) return;
      out.push(
        horizontal
          ? `<text x="${r1(p)}" y="${plot.y + plot.h + 16}" text-anchor="middle" font-size="11" fill="${ink.muted}">${esc(fmt(t))}</text>`
          : `<text x="${plot.x - 8}" y="${r1(p) + 4}" text-anchor="end" font-size="11" fill="${ink.muted}">${esc(fmt(t))}</text>`
      );
    });
  }

  // The zero line reads as an axis, one step darker than the grid.
  const zero = min <= 0 && max >= 0 ? valueToPos(0) : valueToPos(min);
  out.push(
    horizontal
      ? `<line x1="${r1(zero)}" y1="${plot.y}" x2="${r1(zero)}" y2="${plot.y + plot.h}" stroke="${ink.axis}" stroke-width="1"/>`
      : `<line x1="${plot.x}" y1="${r1(zero)}" x2="${plot.x + plot.w}" y2="${r1(zero)}" stroke="${ink.axis}" stroke-width="1"/>`
  );

  const band = (horizontal ? plot.h : plot.w) / Math.max(1, count);
  const bandCenter = (i) => (horizontal ? plot.y : plot.x) + band * (i + 0.5);

  // Category labels, thinned out when they would collide.
  const labelStep = horizontal
    ? 1
    : Math.max(1, Math.ceil((Math.max(...chart.labels.map((l) => textWidth(l, 11)), 10) + 8) / band));
  chart.labels.forEach((label, i) => {
    if (!label || i % labelStep !== 0) return;
    out.push(
      horizontal
        ? `<text x="${plot.x - 8}" y="${r1(bandCenter(i)) + 4}" text-anchor="end" font-size="11" fill="${ink.secondary}">${esc(label)}</text>`
        : `<text x="${r1(bandCenter(i))}" y="${plot.y + plot.h + 18}" text-anchor="middle" font-size="11" fill="${ink.secondary}">${esc(label)}</text>`
    );
  });

  if (chart.type === 'line' || chart.type === 'area') {
    out.push(...renderLines(chart, series, colors, ink, surface, { plot, band, bandCenter, valueToPos, zero, fmt }));
  } else {
    out.push(...renderBars(chart, series, colors, ink, surface, {
      plot, band, count, horizontal, stacked, valueToPos, zero, fmt, min, max,
    }));
  }

  return out;
}

function renderBars(chart, series, colors, ink, surface, opts) {
  const { plot, band, count, horizontal, stacked, valueToPos, zero, fmt } = opts;
  const out = [];

  const groupCount = stacked ? 1 : series.length;
  // Cap the mark and leave the band's leftover as air.
  const slot = Math.min(BAR_MAX, Math.max(3, (band * 0.72) / groupCount));
  const groupWidth = slot * groupCount + GAP * (groupCount - 1);

  const maxIndex = series.map((s) => indexOfExtreme(s.values));

  for (let i = 0; i < count; i++) {
    const center = (horizontal ? plot.y : plot.x) + band * (i + 0.5);
    let positive = 0;
    let negative = 0;

    series.forEach((s, si) => {
      const value = s.values[i];
      if (value === null || value === undefined) return;
      const color = colors[si % colors.length];

      let from;
      let to;
      if (stacked) {
        const base = value >= 0 ? positive : negative;
        from = valueToPos(base);
        to = valueToPos(base + value);
        if (value >= 0) positive += value;
        else negative += value;
      } else {
        from = zero;
        to = valueToPos(value);
      }

      const offset = stacked ? 0 : si * (slot + GAP) - groupWidth / 2 + slot / 2;
      const cross = center + offset;

      const rect = horizontal
        ? { x: Math.min(from, to), y: cross - slot / 2, w: Math.abs(to - from), h: slot }
        : { x: cross - slot / 2, y: Math.min(from, to), w: slot, h: Math.abs(to - from) };

      // 4px rounded data-end, square at the baseline. In SVG y grows downward, so
      // a positive column's data end has the *smaller* y — getting this sign wrong
      // rounds the baseline instead, which is the opposite of the spec.
      const radius = Math.min(4, horizontal ? rect.h / 2 : rect.w / 2, Math.max(0, (horizontal ? rect.w : rect.h) - 1));
      const dataEnd = horizontal ? (to >= from ? 'right' : 'left') : to <= from ? 'top' : 'bottom';
      out.push(
        `<path d="${barPath(rect, radius, dataEnd)}" fill="${color}">` +
          `<title>${esc(`${s.name || '계열'} · ${chart.labels[i] ?? i + 1}: ${fmt(value)}`)}</title>` +
          '</path>'
      );

      // Direct labels, selectively: the extreme of each series, or all when asked.
      const wants =
        chart.options.valueLabels === 'all' ||
        (chart.options.valueLabels === 'max' && maxIndex[si] === i) ||
        (chart.options.valueLabels === 'ends' && (i === 0 || i === count - 1));
      if (!wants || stacked) return;

      const text = fmt(value);
      const tw = textWidth(text, 11);
      if (horizontal) {
        const x = to >= from ? rect.x + rect.w + 4 : rect.x - 4 - tw;
        if (x >= plot.x - 2 && x + tw <= plot.x + plot.w + 16) {
          out.push(
            `<text x="${r1(x)}" y="${r1(rect.y + rect.h / 2 + 4)}" font-size="11" fill="${ink.secondary}">${esc(text)}</text>`
          );
        }
      } else {
        // Inside the bar when it is tall enough to hold the label with padding,
        // otherwise just outside the data end — never clipped.
        const growsUp = dataEnd === 'top';
        const inside = rect.h >= 22;
        const placeY = inside
          ? growsUp ? rect.y + 15 : rect.y + rect.h - 6
          : growsUp ? rect.y - 5 : rect.y + rect.h + 13;
        if (placeY > plot.y + 2 && placeY < plot.y + plot.h + 20) {
          out.push(
            `<text x="${r1(rect.x + rect.w / 2)}" y="${r1(placeY)}" text-anchor="middle" font-size="11" fill="${
              inside ? '#ffffff' : ink.secondary
            }">${esc(text)}</text>`
          );
        }
      }
    });
  }

  return out;
}

/**
 * A bar with its two data-end corners rounded and the baseline corners square.
 * `dataEnd` names the side the value grows toward.
 */
function barPath(rect, radius, dataEnd) {
  const { x, y, w, h } = rect;
  const r = Math.max(0, Math.min(radius, w / 2, h / 2));
  if (r === 0) return `M${r1(x)} ${r1(y)}h${r1(w)}v${r1(h)}h${r1(-w)}z`;

  const X = (v) => r1(x + v);
  const Y = (v) => r1(y + v);

  switch (dataEnd) {
    case 'top':
      return `M${X(0)} ${Y(h)}V${Y(r)}Q${X(0)} ${Y(0)} ${X(r)} ${Y(0)}H${X(w - r)}Q${X(w)} ${Y(0)} ${X(w)} ${Y(r)}V${Y(h)}Z`;
    case 'bottom':
      return `M${X(0)} ${Y(0)}V${Y(h - r)}Q${X(0)} ${Y(h)} ${X(r)} ${Y(h)}H${X(w - r)}Q${X(w)} ${Y(h)} ${X(w)} ${Y(h - r)}V${Y(0)}Z`;
    case 'right':
      return `M${X(0)} ${Y(0)}H${X(w - r)}Q${X(w)} ${Y(0)} ${X(w)} ${Y(r)}V${Y(h - r)}Q${X(w)} ${Y(h)} ${X(w - r)} ${Y(h)}H${X(0)}Z`;
    default:
      return `M${X(w)} ${Y(0)}H${X(r)}Q${X(0)} ${Y(0)} ${X(0)} ${Y(r)}V${Y(h - r)}Q${X(0)} ${Y(h)} ${X(r)} ${Y(h)}H${X(w)}Z`;
  }
}

function renderLines(chart, series, colors, ink, surface, opts) {
  const { plot, bandCenter, valueToPos, zero, fmt } = opts;
  const out = [];
  const isArea = chart.type === 'area';

  series.forEach((s, si) => {
    const color = colors[si % colors.length];
    const points = s.values
      .map((v, i) => (v === null || v === undefined ? null : { x: bandCenter(i), y: valueToPos(v), v, i }))
      .filter(Boolean);
    if (!points.length) return;

    if (isArea) {
      // A wash, never a saturated block.
      const d = `M${r1(points[0].x)} ${r1(zero)}` +
        points.map((p) => `L${r1(p.x)} ${r1(p.y)}`).join('') +
        `L${r1(points[points.length - 1].x)} ${r1(zero)}z`;
      out.push(`<path d="${d}" fill="${color}" fill-opacity="0.1"/>`);
    }

    const path = points.map((p, i) => `${i === 0 ? 'M' : 'L'}${r1(p.x)} ${r1(p.y)}`).join('');
    out.push(
      `<path d="${path}" fill="none" stroke="${color}" stroke-width="${LINE_W}" stroke-linejoin="round" stroke-linecap="round"/>`
    );

    // Markers carry a 2px surface ring so overlapping series stay readable.
    for (const p of points) {
      out.push(
        `<circle cx="${r1(p.x)}" cy="${r1(p.y)}" r="${MARKER_R}" fill="${color}" stroke="${surface}" stroke-width="2">` +
          `<title>${esc(`${s.name || '계열'} · ${chart.labels[p.i] ?? p.i + 1}: ${fmt(p.v)}`)}</title>` +
          '</circle>'
      );
    }

    const wants = chart.options.valueLabels;
    const marked =
      wants === 'all'
        ? points
        : wants === 'ends'
        ? [points[0], points[points.length - 1]]
        : wants === 'max'
        ? [points.reduce((best, p) => (Math.abs(p.v) > Math.abs(best.v) ? p : best), points[0])]
        : [];

    for (const p of marked) {
      if (!p) continue;
      const text = fmt(p.v);
      const tw = textWidth(text, 11);
      const x = Math.min(Math.max(plot.x + tw / 2, p.x), plot.x + plot.w - tw / 2);
      const y = p.y - 10 > plot.y ? p.y - 10 : p.y + 18;
      out.push(
        `<text x="${r1(x)}" y="${r1(y)}" text-anchor="middle" font-size="11" fill="${ink.secondary}">${esc(text)}</text>`
      );
    }
  });

  return out;
}

/* ------------------------------------------------------------------- pie */

function renderPie(chart, series, colors, ink, surface, { w, top, bottom }) {
  const values = (series[0]?.values ?? []).map((v) => (v === null ? 0 : Math.abs(v)));
  const total = values.reduce((a, b) => a + b, 0);
  const out = [];
  if (!total) return out;

  const size = Math.min(w - 40, bottom - top - 10);
  const radius = size / 2;
  const cx = w / 2;
  const cy = top + size / 2;
  const inner = chart.type === 'donut' ? radius * 0.58 : 0;
  const fmt = (v) => formatValue(v, chart.options.numberFormat);

  let angle = -Math.PI / 2;
  values.forEach((value, i) => {
    if (!value) return;
    const sweep = (value / total) * Math.PI * 2;
    const color = colors[i % colors.length];
    out.push(
      `<path d="${arcPath(cx, cy, radius, inner, angle, angle + sweep)}" fill="${color}" stroke="${surface}" stroke-width="${GAP}">` +
        `<title>${esc(`${chart.labels[i] ?? `항목 ${i + 1}`}: ${fmt(value)} (${((value / total) * 100).toFixed(1)}%)`)}</title>` +
        '</path>'
    );

    // Label slices big enough to hold one, others rely on the legend.
    const share = value / total;
    if (share >= 0.08) {
      const mid = angle + sweep / 2;
      const lr = inner ? (radius + inner) / 2 : radius * 0.62;
      out.push(
        `<text x="${r1(cx + Math.cos(mid) * lr)}" y="${r1(cy + Math.sin(mid) * lr + 4)}" text-anchor="middle" font-size="11" font-weight="600" fill="#ffffff">${esc(
          `${Math.round(share * 100)}%`
        )}</text>`
      );
    }
    angle += sweep;
  });

  if (chart.type === 'donut') {
    out.push(
      `<text x="${r1(cx)}" y="${r1(cy - 2)}" text-anchor="middle" font-size="18" font-weight="600" fill="${ink.primary}">${esc(fmt(total))}</text>`,
      `<text x="${r1(cx)}" y="${r1(cy + 16)}" text-anchor="middle" font-size="11" fill="${ink.muted}">합계</text>`
    );
  }

  return out;
}

function arcPath(cx, cy, radius, inner, from, to) {
  const large = to - from > Math.PI ? 1 : 0;
  const x1 = cx + Math.cos(from) * radius;
  const y1 = cy + Math.sin(from) * radius;
  const x2 = cx + Math.cos(to) * radius;
  const y2 = cy + Math.sin(to) * radius;

  if (!inner) {
    return `M${r1(cx)} ${r1(cy)}L${r1(x1)} ${r1(y1)}A${r1(radius)} ${r1(radius)} 0 ${large} 1 ${r1(x2)} ${r1(y2)}z`;
  }
  const ix1 = cx + Math.cos(to) * inner;
  const iy1 = cy + Math.sin(to) * inner;
  const ix2 = cx + Math.cos(from) * inner;
  const iy2 = cy + Math.sin(from) * inner;
  return (
    `M${r1(x1)} ${r1(y1)}A${r1(radius)} ${r1(radius)} 0 ${large} 1 ${r1(x2)} ${r1(y2)}` +
    `L${r1(ix1)} ${r1(iy1)}A${r1(inner)} ${r1(inner)} 0 ${large} 0 ${r1(ix2)} ${r1(iy2)}z`
  );
}

/* ---------------------------------------------------------------- legend */

function renderLegend(entries, ink, { w, y }) {
  const out = [];
  const gap = 16;
  const widths = entries.map((e) => 12 + 5 + textWidth(e.name, 11) + gap);
  const total = widths.reduce((a, b) => a + b, 0) - gap;
  let x = Math.max(8, (w - total) / 2);

  entries.forEach((entry, i) => {
    out.push(`<rect x="${r1(x)}" y="${r1(y - 9)}" width="10" height="10" rx="2" fill="${entry.color}"/>`);
    // Text wears ink tokens, never the series colour.
    out.push(
      `<text x="${r1(x + 15)}" y="${r1(y)}" font-size="11" fill="${ink.secondary}">${esc(entry.name)}</text>`
    );
    x += widths[i];
  });
  return out;
}

/* --------------------------------------------------------------- scaling */

/** A single value axis with "nice" ticks. Never a second scale. */
function scaleFor(series, stacked, type) {
  let min = 0;
  let max = 0;

  if (stacked) {
    const length = Math.max(...series.map((s) => s.values.length), 0);
    for (let i = 0; i < length; i++) {
      let positive = 0;
      let negative = 0;
      for (const s of series) {
        const v = s.values[i];
        if (v === null || v === undefined) continue;
        if (v >= 0) positive += v;
        else negative += v;
      }
      max = Math.max(max, positive);
      min = Math.min(min, negative);
    }
  } else {
    for (const s of series) {
      for (const v of s.values) {
        if (v === null || v === undefined) continue;
        max = Math.max(max, v);
        min = Math.min(min, v);
      }
    }
  }

  // Line charts read better without a forced zero when values sit far from it.
  if (type === 'line' && min > 0 && min > (max - min) * 1.5) {
    const pad = (max - min) * 0.15 || Math.abs(max) * 0.1 || 1;
    min -= pad;
  }
  if (max === min) {
    max = min + (Math.abs(min) || 1);
  }

  const step = niceStep((max - min) / 5);
  const niceMin = Math.floor(min / step) * step;
  const niceMax = Math.ceil(max / step) * step;
  const ticks = [];
  for (let t = niceMin; t <= niceMax + step / 2; t += step) ticks.push(round12(t));

  return { min: niceMin, max: niceMax, ticks };
}

/**
 * Choose which tick labels to draw, by measuring instead of by every-nth.
 *
 * Walks from the first tick keeping any label that clears the last kept one, then
 * guarantees the final tick — dropping the neighbour it would have collided with —
 * so the axis always states its full range without any pair overlapping.
 */
function selectTickLabels(ticks, position, measure) {
  const last = ticks.length - 1;
  if (last < 0) return new Set();
  if (last === 0) return new Set([0]);

  const span = (i) => {
    const center = position(ticks[i]);
    const half = measure(ticks[i]) / 2 + 3;
    return [center - half, center + half];
  };

  const kept = [0];
  let [, prevEnd] = span(0);
  for (let i = 1; i < last; i++) {
    const [start, end] = span(i);
    if (Math.min(start, end) > Math.max(prevEnd, prevEnd)) {
      kept.push(i);
      prevEnd = Math.max(start, end);
    }
  }

  // The last tick always earns its place; evict anything it overlaps.
  const [lastStart, lastEnd] = span(last);
  const lastMin = Math.min(lastStart, lastEnd);
  const lastMax = Math.max(lastStart, lastEnd);
  const survivors = kept.filter((i) => {
    const [s, e] = span(i);
    return Math.max(s, e) < lastMin || Math.min(s, e) > lastMax;
  });
  // Never drop the first label; if it collides with the last, the axis is tiny and
  // stating the range beats stating the origin.
  survivors.push(last);
  return new Set(survivors);
}

function niceStep(raw) {
  if (!Number.isFinite(raw) || raw <= 0) return 1;
  const magnitude = 10 ** Math.floor(Math.log10(raw));
  const scaled = raw / magnitude;
  const step = scaled <= 1 ? 1 : scaled <= 2 ? 2 : scaled <= 5 ? 5 : 10;
  return step * magnitude;
}

function round12(n) {
  return Number(Number(n).toPrecision(12));
}

/* --------------------------------------------------------------- helpers */

function formatValue(v, fmt) {
  if (v === null || v === undefined) return '';
  if (fmt) return applyNumFmt(v, fmt);
  const abs = Math.abs(v);
  if (abs >= 1e8) return `${round12(v / 1e8)}억`;
  if (abs >= 1e4) return `${round12(v / 1e4)}만`;
  if (abs >= 1000) return v.toLocaleString('ko-KR', { maximumFractionDigits: 1 });
  return String(round12(Number(v.toFixed(2))));
}

/** Rough advance width; CJK glyphs are about a full em, latin about half. */
function textWidth(text, size) {
  let units = 0;
  for (const ch of String(text ?? '')) units += /[ᄀ-ᇿ　-〿㄰-㆏가-힯一-鿿＀-￯]/.test(ch) ? 1 : 0.55;
  return units * size;
}

function r1(n) {
  return Math.round(n * 10) / 10;
}

function esc(text) {
  return String(text ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function isDark(color) {
  const hex = String(color ?? '#ffffff').replace('#', '');
  const full = hex.length === 3 ? hex.split('').map((c) => c + c).join('') : hex;
  if (!/^[0-9a-fA-F]{6}$/.test(full)) return false;
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16) / 255);
  const lin = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b) < 0.45;
}

function indexOfExtreme(values) {
  let best = -1;
  let bestValue = -Infinity;
  values.forEach((v, i) => {
    if (v === null || v === undefined) return;
    if (Math.abs(v) > bestValue) {
      bestValue = Math.abs(v);
      best = i;
    }
  });
  return best;
}

export { normalizeChartSpec, resolveChartSpec };
