/**
 * The WYSIWYG bridge: what the writer sees is the rendered markdown, and the
 * stored text is recovered from the DOM the browser edited.
 *
 * Every block in this app is a markdown string, but a user who does not know
 * markdown should never have to meet `#` or `**`. The editor renders the block
 * and lets the browser edit the rendered DOM; `domToMd` walks that DOM back
 * into the markdown the renderer came from, so the file on disk stays clean
 * and portable.
 */

import { renderMarkdown } from './markdown.js';

/** Invisible filler that keeps a bare list marker a list when rendered. */
export const ZWSP = '\u200B';
/** Private-use marker used to find a caret position inside markdown text. */
const CARET = '\uFFFF';

// SHOW_TEXT, without depending on the global existing in every DOM.
const SHOW_TEXT = 4;

const BLOCK_TAGS = new Set(['h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'p', 'ul', 'ol', 'li', 'blockquote', 'pre', 'hr', 'table']);

/* ----------------------------------------------------------- DOM → markdown */

/** Recover the markdown that produced the edited DOM of one block. */
export function domToMd(root) {
  return serializeChildren(root).replace(/[\u200B\uFFFF]/g, '');
}

/** Serialize a container's children as block markdown. */
function serializeChildren(node) {
  const parts = [];
  let inline = '';
  for (const child of node.childNodes) {
    if (child.nodeType === 3) {
      if (!child.data.trim()) continue; // inter-block whitespace
      inline += child.data;
      continue;
    }
    if (child.nodeType !== 1) continue;
    const tag = child.tagName.toLowerCase();
    if (BLOCK_TAGS.has(tag)) {
      if (inline.trim()) parts.push(inline.trim());
      inline = '';
      parts.push(serializeBlock(child));
    } else {
      inline += inlineOf(child);
    }
  }
  if (inline.trim()) parts.push(inline.trim());
  return parts.join('\n\n');
}

function serializeBlock(node) {
  const tag = node.tagName.toLowerCase();
  if (tag === 'p') return inlineOf(node);
  if (tag[0] === 'h' && tag.length === 2 && tag >= 'h1' && tag <= 'h6') {
    return `${'#'.repeat(Number(tag[1]))} ${inlineOf(node)}`;
  }
  if (tag === 'ul' || tag === 'ol') return serializeList(node);
  if (tag === 'li') return serializeListItem(node);
  if (tag === 'blockquote') {
    return serializeChildren(node)
      .split('\n')
      .map((line) => `> ${line}`)
      .join('\n');
  }
  if (tag === 'hr') return '---';
  if (tag === 'pre') {
    const code = node.querySelector('code') ?? node;
    const text = (code.textContent ?? '').replace(/\n$/, '');
    const lang = /(?:^|\s)language-([\w+-]+)/.exec(code.className ?? '');
    return `\`\`\`${lang ? lang[1] : ''}\n${text}\n\`\`\``;
  }
  if (tag === 'table') return serializeTable(node);
  return inlineOf(node);
}

/**
 * A markdown table back from its rendered form.
 *
 * Tables ride inside text blocks (a deck's fact box, a mixed paragraph), and a
 * round trip that flattened them would destroy data on the first keystroke.
 * Cells keep their inline marks; a line break inside a cell stays a literal
 * `<br>` because a real newline would tear the row apart.
 */
function serializeTable(node) {
  const rowsOf = (parent) =>
    [...parent.querySelectorAll('tr')].map((tr) =>
      [...tr.children]
        .filter((c) => /^(td|th)$/i.test(c.tagName))
        .map((cell) => inlineOf(cell, true).trim().replace(/\|/g, '\\|').replace(/\n/g, ' '))
    );

  const head = node.querySelector('thead');
  const aligns = [];
  let header = null;
  // marked records column alignment as an `align` attribute (GFM의 :--: · --:).
  const alignOf = (el) => {
    const a = el.getAttribute('align')?.toLowerCase();
    if (a === 'center') return ':---:';
    if (a === 'right') return '---:';
    if (a === 'left') return ':---';
    const style = el.getAttribute('style') ?? '';
    if (/text-align\s*:\s*center/i.test(style)) return ':---:';
    if (/text-align\s*:\s*right/i.test(style)) return '---:';
    if (/text-align\s*:\s*left/i.test(style)) return ':---';
    return '---';
  };
  if (head) {
    const tr = head.querySelector('tr');
    if (tr) {
      header = [...tr.children]
        .filter((c) => /^(td|th)$/i.test(c.tagName))
        .map((cell) => inlineOf(cell, true).trim().replace(/\|/g, '\\|').replace(/\n/g, ' '));
      for (const th of tr.children) aligns.push(alignOf(th));
    }
  }
  const body = node.querySelector('tbody');
  const bodyRows = body
    ? rowsOf(body)
    : [...node.querySelectorAll('tr')]
        .filter((tr) => !tr.closest('thead'))
        .map((tr) =>
          [...tr.children]
            .filter((c) => /^(td|th)$/i.test(c.tagName))
            .map((cell) => inlineOf(cell, true).trim().replace(/\|/g, '\\|').replace(/\n/g, ' '))
        );
  if (!header && bodyRows.length) header = bodyRows.shift();
  if (!header) return '';

  const width = Math.max(header.length, ...bodyRows.map((r) => r.length), 1);
  const pad = (row) => {
    const r = [...row];
    while (r.length < width) r.push('');
    return r;
  };
  const line = (row) => `| ${row.join(' | ')} |`;
  const out = [line(pad(header))];
  out.push(`|${Array.from({ length: width }, (_, i) => aligns[i] ?? '---').join('|')}|`);
  for (const row of bodyRows) out.push(line(pad(row)));
  return out.join('\n');
}

function serializeList(node) {
  const ordered = node.tagName.toLowerCase() === 'ol';
  const start = ordered ? Number(node.getAttribute('start') ?? 1) || 1 : 1;
  let n = start;
  const items = [];
  for (const li of node.children) {
    if (li.tagName.toLowerCase() !== 'li') continue;
    items.push(serializeListItem(li, ordered ? `${n++}. ` : null));
  }
  return items.join('\n');
}

function serializeListItem(li, forcedMarker = null) {
  const depth = (() => {
    let d = -1;
    let parent = li.parentElement;
    while (parent && (parent.tagName === 'UL' || parent.tagName === 'OL')) {
      d++;
      parent = parent.parentElement?.parentElement ?? null;
    }
    return Math.max(0, d);
  })();

  let text = '';
  let checked = null;
  let nested = null;
  for (const child of li.childNodes) {
    if (child.nodeType === 3) {
      if (child.data.trim() === '') continue;
      text += child.data;
      continue;
    }
    if (child.nodeType !== 1) continue;
    const tag = child.tagName.toLowerCase();
    if (tag === 'input') {
      checked = !!child.checked;
      continue;
    }
    if (tag === 'ul' || tag === 'ol') {
      nested = child;
      continue;
    }
    if (BLOCK_TAGS.has(tag)) {
      text += `\n${serializeBlock(child)}`;
      continue;
    }
    text += inlineOf(child);
  }

  const check = checked === null ? '' : checked ? '[x] ' : '[ ] ';
  const marker = checked === null ? forcedMarker ?? '- ' : '- ';
  let md = `${'  '.repeat(depth)}${marker}${check}${text.trim()}`;
  if (nested) md += `\n${serializeList(nested)}`;
  return md;
}

/** Inline markdown of one element: text plus emphasis, links and breaks. */
function inlineOf(node, inCell = false) {
  if (node.nodeType === 3) return node.data;
  if (node.nodeType !== 1) return '';
  const tag = node.tagName.toLowerCase();
  if (tag === 'br') return inCell ? '<br>' : '\n';
  if (tag === 'strong' || tag === 'b') return `**${inlineOfChildren(node, inCell)}**`;
  if (tag === 'em' || tag === 'i') return `*${inlineOfChildren(node, inCell)}*`;
  if (tag === 'del' || tag === 's' || tag === 'strike') return `~~${inlineOfChildren(node, inCell)}~~`;
  if (tag === 'code') return `\`${inlineOfChildren(node, inCell)}\``;
  if (tag === 'a') return `[${inlineOfChildren(node, inCell)}](${node.getAttribute('href') ?? ''})`;
  if (tag === 'img') return `![${node.getAttribute('alt') ?? ''}](${node.getAttribute('src') ?? ''})`;
  return inlineOfChildren(node, inCell); // p, h1…, mark, span, u — plain passthrough
}

function inlineOfChildren(node, inCell = false) {
  let out = '';
  for (const child of node.childNodes) out += inlineOf(child, inCell);
  return out;
}

/** Wrap or unwrap the selection in one inline element (Ctrl+B / Ctrl+I). */
export function toggleInlineWrap(root, tag) {
  const sel = window.getSelection();
  if (!sel || !sel.rangeCount || !root.contains(sel.anchorNode)) return false;
  const range = sel.getRangeAt(0);

  if (range.collapsed) {
    const wrap = document.createElement(tag);
    wrap.textContent = '텍스트';
    range.insertNode(wrap);
    const inner = document.createRange();
    inner.selectNodeContents(wrap);
    sel.removeAllRanges();
    sel.addRange(inner);
    return true;
  }

  // The whole selection already inside one wrapper → unwrap it.
  let container = range.commonAncestorContainer;
  if (container.nodeType === 3) container = container.parentElement;
  const existing = container instanceof Element ? container.closest(tag) : null;
  if (existing?.parentElement && range.toString() === existing.textContent) {
    const parent = existing.parentElement;
    const kids = [...existing.childNodes];
    for (const kid of kids) parent.insertBefore(kid, existing);
    const next = document.createRange();
    if (kids.length) {
      next.setStartBefore(kids[0]);
      next.setEndAfter(kids[kids.length - 1]);
    } else {
      next.setStartBefore(existing);
      next.setEndBefore(existing);
    }
    parent.removeChild(existing);
    sel.removeAllRanges();
    sel.addRange(next);
    return true;
  }

  const fragment = range.extractContents();
  const wrap = document.createElement(tag);
  wrap.appendChild(fragment);
  range.insertNode(wrap);
  const inner = document.createRange();
  inner.selectNodeContents(wrap);
  sel.removeAllRanges();
  sel.addRange(inner);
  return true;
}

/* ------------------------------------------------------------- caret maths */

function textOf(node) {
  if (node.nodeType === 3) return node.data.length;
  if (node.nodeType !== 1) return 0;
  let sum = 0;
  const walker = document.createTreeWalker(node, SHOW_TEXT);
  let n;
  while ((n = walker.nextNode())) sum += n.data.length;
  return sum;
}

/** Total visible text length of a container. */
export function textLength(root) {
  let sum = 0;
  const walker = document.createTreeWalker(root, SHOW_TEXT);
  let n;
  while ((n = walker.nextNode())) sum += n.data.length;
  return sum;
}

/** The caret as an offset into all the block's visible text. */
export function caretTextOffset(root) {
  const sel = window.getSelection();
  if (!sel || !sel.rangeCount) return textLength(root);
  const range = sel.getRangeAt(0);
  const node = range.startContainer;
  if (!root.contains(node)) return textLength(root);

  let sum = 0;
  const walker = document.createTreeWalker(root, SHOW_TEXT);
  let n = walker.nextNode();
  while (n) {
    if (n === node) return sum + range.startOffset;
    if (n.compareDocumentPosition(node) & Node.DOCUMENT_POSITION_FOLLOWING) sum += n.data.length;
    else break; // n sits at or after the anchor
    n = walker.nextNode();
  }
  // The anchor is an element: count the text of its children up to the index.
  if (node.nodeType === 1) {
    let child = node.firstChild;
    for (let i = 0; child && i < range.startOffset; i++) {
      sum += textOf(child);
      child = child.nextSibling;
    }
  }
  return sum;
}

/** Put the caret at a plain-text offset inside the block. */
export function setCaretAtTextOffset(root, offset) {
  const target = Math.max(0, offset | 0);
  const walker = document.createTreeWalker(root, SHOW_TEXT);
  let remaining = target;
  let n = walker.nextNode();
  while (n) {
    if (remaining <= n.data.length) {
      placeCaret(root, n, remaining);
      return;
    }
    remaining -= n.data.length;
    n = walker.nextNode();
  }
  // Past the end — caret after the last text node; no text at all — block start.
  const sel = window.getSelection();
  const range = document.createRange();
  const last = walker.previousNode();
  if (last) {
    range.setStart(last, last.data.length);
    range.collapse(true);
  } else {
    range.selectNodeContents(root);
    range.collapse(true);
  }
  sel?.removeAllRanges();
  sel?.addRange(range);
}

function placeCaret(root, node, offset) {
  const sel = window.getSelection();
  const range = document.createRange();
  range.setStart(node, offset);
  range.collapse(true);
  sel?.removeAllRanges();
  sel?.addRange(range);
}

/** The caret as an offset into the block's markdown. */
export function caretMdOffset(root) {
  const sel = window.getSelection();
  if (!sel || !sel.rangeCount || !root.contains(sel.anchorNode)) return 0;
  const range = sel.getRangeAt(0);
  const probe = range.cloneRange();
  probe.collapse(true);
  const marker = document.createTextNode(CARET);
  probe.insertNode(marker);
  const md = domToMd(root);
  const at = md.indexOf(CARET);
  marker.remove();
  sel.removeAllRanges();
  sel.addRange(range);
  return at < 0 ? Math.max(0, md.length - 1) : at;
}

/**
 * Where a given markdown offset lands in the rendered text.
 *
 * The probe renders the markdown with an invisible marker at that offset and
 * reports the marker's own text offset — that is where the caret belongs after
 * the document re-renders (Enter inside a list, indenting, …).
 */
export function textOffsetForMd(md, mdOffset) {
  const source = String(md ?? '');
  const offset = Math.max(0, Math.min(mdOffset | 0, source.length));
  const probe = `${source.slice(0, offset)}${ZWSP}${CARET}${source.slice(offset)}`;
  const holder = document.createElement('div');
  holder.innerHTML = renderMarkdown(probe);
  let sum = 0;
  const walker = document.createTreeWalker(holder, SHOW_TEXT);
  let n;
  while ((n = walker.nextNode())) {
    const at = n.data.indexOf(CARET);
    if (at !== -1) return sum + at;
    sum += n.data.length;
  }
  return sum;
}