import { marked } from 'marked';
import DOMPurify from 'dompurify';

marked.setOptions({ gfm: true, breaks: true });

/**
 * Lines that are only a marker (`- `, `1. `, `> `) do not become a list or quote
 * in CommonMark — they render as literal text, which is exactly the raw syntax
 * a non-technical writer would see and delete. The same goes for a lone
 * leading/trailing newline: marked folds it away, but that is the line the
 * caret is sitting on after Enter. Give each an invisible occupant so the
 * structure survives everywhere markdown is drawn.
 */
export function normalizeMd(md) {
  let s = String(md ?? '').replace(
    /^[ \t]*((?:[-+*]|\d+[.)])[ \t]+|>[ \t]*)[\u200B\uFFFF]*$/gm,
    (m) => `${m}\u200B`
  );
  if (/^\n/.test(s) && !/^\n\n/.test(s)) s = `\u200B${s}`;
  if (/\n$/.test(s) && !/\n\n$/.test(s)) s = `${s}\u200B`;
  return s;
}

/** The markdown line around an offset, split at the caret. */
export function mdLineAt(md, offset) {
  const source = String(md ?? '');
  const lineStart = source.lastIndexOf('\n', offset - 1) + 1;
  const nl = source.indexOf('\n', offset);
  const lineEnd = nl === -1 ? source.length : nl;
  return {
    lineStart,
    lineEnd,
    before: source.slice(lineStart, offset),
    after: source.slice(offset, lineEnd),
  };
}

const stripMarker = (s) => s.replace(/^[ \t]*(?:>|(?:[-+*]|\d+[.)]))[ \t]?/, '');

/**
 * True when the caret sits on an empty line — the Enter that splits the block
 * into two paragraphs. A lone quote/list marker counts as empty.
 */
export function mdLineEmptyAt(md, offset) {
  const { before, after } = mdLineAt(md, offset);
  return stripMarker(before).trim() === '' && stripMarker(after).trim() === '';
}

/**
 * A heading typed below the block's first line — marked renders it fine in
 * place, but the outline and anchors treat headings as block boundaries, so it
 * becomes its own block. Returns the line's offset, or -1.
 */
export function mdMidHeadingOffset(md) {
  const source = String(md ?? '');
  let offset = 0;
  const lines = source.split('\n');
  for (let i = 0; i < lines.length; i++) {
    if (i > 0 && /^#{1,6} /.test(lines[i])) return offset;
    offset += lines[i].length + 1;
  }
  return -1;
}

/**
 * Render block markdown to sanitized HTML.
 *
 * Sanitising matters even for local files: a project folder can be shared, and a
 * pasted `<img onerror>` should not execute in the editor.
 *
 * `assetResolver` maps a project-relative image path to a URL the browser can
 * load. The markdown keeps the relative path — that is what makes the folder
 * portable — so the rewrite happens at render time, never in the stored text.
 */
export function renderMarkdown(md, { assetResolver } = {}) {
  const source = normalizeMd(md);
  if (!source.trim()) return '';
  try {
    const html = DOMPurify.sanitize(marked.parse(source), { USE_PROFILES: { html: true } });
    return assetResolver ? rewriteAssetUrls(html, assetResolver) : html;
  } catch {
    return DOMPurify.sanitize(`<pre>${escapeHtml(source)}</pre>`);
  }
}

/** Point <img src> at the asset endpoint for paths inside the project. */
function rewriteAssetUrls(html, resolve) {
  return html.replace(/<img\b([^>]*?)\bsrc="([^"]*)"([^>]*)>/gi, (match, before, src, after) => {
    const url = resolve(src);
    if (!url || url === src) return match;
    return `<img${before}src="${escapeHtml(url)}"${after}>`;
  });
}

export function escapeHtml(text) {
  return String(text ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/* ------------------------------------------------- inline markdown editing */

/**
 * Wrap the current textarea selection in a markdown marker, or unwrap it when
 * already wrapped — the behaviour a user expects from a Bold button.
 * Returns the next value plus the selection to restore.
 */
export function toggleWrap(value, start, end, marker) {
  const before = value.slice(0, start);
  const selected = value.slice(start, end);
  const after = value.slice(end);
  const len = marker.length;

  if (selected.startsWith(marker) && selected.endsWith(marker) && selected.length >= len * 2) {
    const inner = selected.slice(len, -len);
    return { value: before + inner + after, start, end: start + inner.length };
  }
  if (before.endsWith(marker) && after.startsWith(marker)) {
    return {
      value: before.slice(0, -len) + selected + after.slice(len),
      start: start - len,
      end: end - len,
    };
  }
  const next = `${marker}${selected || '텍스트'}${marker}`;
  return { value: before + next + after, start: start + len, end: start + next.length - len };
}

/** Toggle a line prefix (`# `, `- `, `> `) across every selected line. */
export function toggleLinePrefix(value, start, end, prefix) {
  const lineStart = value.lastIndexOf('\n', start - 1) + 1;
  const lineEndIdx = value.indexOf('\n', end);
  const lineEnd = lineEndIdx === -1 ? value.length : lineEndIdx;

  const target = value.slice(lineStart, lineEnd);
  const lines = target.split('\n');
  // Any heading level counts as "already a heading" so H1 -> H2 replaces cleanly.
  const family = prefix.trim().startsWith('#') ? /^#{1,6} / : new RegExp(`^${escapeRe(prefix)}`);
  const allHave = lines.every((l) => l.startsWith(prefix));

  const next = lines
    .map((l) => (allHave ? l.slice(prefix.length) : `${prefix}${l.replace(family, '')}`))
    .join('\n');

  const value2 = value.slice(0, lineStart) + next + value.slice(lineEnd);
  const delta = next.length - target.length;
  return { value: value2, start: lineStart, end: Math.max(lineStart, end + delta) };
}

function escapeRe(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * Indent or outdent the selected lines by one level.
 *
 * Word's Tab at a list item demotes it; in markdown a level is two spaces of
 * leading whitespace, and that is what a nested bullet is on disk. Returns
 * `null` when there is nothing to outdent, so the caller can fall back to the
 * paragraph's own indent.
 */
export function indentLines(value, start, end, outdent) {
  const lineStart = value.lastIndexOf('\n', start - 1) + 1;
  const lineEndIdx = value.indexOf('\n', end);
  const lineEnd = lineEndIdx === -1 ? value.length : lineEndIdx;
  const target = value.slice(lineStart, lineEnd);
  const lines = target.split('\n');

  if (outdent && !lines.some((l) => /^ {1,2}/.test(l))) return null;

  const next = lines
    .map((l) => (outdent ? l.replace(/^ {1,2}/, '') : `  ${l}`))
    .join('\n');
  const value2 = value.slice(0, lineStart) + next + value.slice(lineEnd);
  const delta = next.length - target.length;
  return { value: value2, start: Math.max(lineStart, start + (outdent ? -2 : 2)), end: Math.max(lineStart, end + delta) };
}

/** True when the line the caret sits on is a bullet or numbered item. */
export function isListLine(value, caret) {
  const lineStart = value.lastIndexOf('\n', caret - 1) + 1;
  const lineEndIdx = value.indexOf('\n', lineStart);
  const line = value.slice(lineStart, lineEndIdx === -1 ? value.length : lineEndIdx);
  return /^[ \t]*([-+*]|\d+[.)])[ \t]+/.test(line);
}

/** Ordered-list aware continuation when the user presses Enter mid-list. */
export function continueList(value, caret) {
  const lineStart = value.lastIndexOf('\n', caret - 1) + 1;
  const line = value.slice(lineStart, caret);

  const bullet = line.match(/^([ \t]*)([-+*])[ \t]+(.*)$/);
  if (bullet) {
    if (!bullet[3].trim()) return { value: value.slice(0, lineStart) + value.slice(caret), caret: lineStart };
    const insert = `\n${bullet[1]}${bullet[2]} `;
    return { value: value.slice(0, caret) + insert + value.slice(caret), caret: caret + insert.length };
  }

  const numbered = line.match(/^([ \t]*)(\d+)([.)])[ \t]+(.*)$/);
  if (numbered) {
    if (!numbered[4].trim()) return { value: value.slice(0, lineStart) + value.slice(caret), caret: lineStart };
    const insert = `\n${numbered[1]}${Number(numbered[2]) + 1}${numbered[3]} `;
    return { value: value.slice(0, caret) + insert + value.slice(caret), caret: caret + insert.length };
  }

  return null;
}

/** Wrap search hits in the rendered HTML, skipping tag interiors. */
export function markHtml(html, query) {
  if (!query) return html;
  const parts = html.split(/(<[^>]*>)/);
  const needle = query.toLowerCase();
  return parts
    .map((part) => {
      if (part.startsWith('<')) return part;
      let out = '';
      let rest = part;
      for (;;) {
        const at = rest.toLowerCase().indexOf(needle);
        if (at === -1) {
          out += rest;
          break;
        }
        out += `${rest.slice(0, at)}<mark class="findhit">${rest.slice(at, at + query.length)}</mark>`;
        rest = rest.slice(at + query.length);
      }
      return out;
    })
    .join('');
}

/* --------------------------------------------------------- syntax colouring */

/** Lightweight highlighter for the file inspector. */
export function highlight(text, language) {
  const src = escapeHtml(text);
  if (language === 'json') {
    return src
      .replace(/(&quot;(?:[^&]|&(?!quot;))*?&quot;)(\s*:)/g, '<span class="tok-key">$1</span>$2')
      .replace(/:\s*(&quot;(?:[^&]|&(?!quot;))*?&quot;)/g, ': <span class="tok-str">$1</span>')
      .replace(/\b(-?\d+\.?\d*(?:e[+-]?\d+)?)\b/gi, '<span class="tok-num">$1</span>')
      .replace(/\b(true|false|null)\b/g, '<span class="tok-num">$1</span>');
  }
  return src
    .replace(/(&lt;!--[\s\S]*?--&gt;)/g, '<span class="tok-cmt">$1</span>')
    .replace(/^(#{1,6} .*)$/gm, '<span class="tok-head">$1</span>')
    .replace(/^(---)$/gm, '<span class="tok-fence">$1</span>')
    .replace(/(`[^`\n]+`)/g, '<span class="tok-str">$1</span>');
}
