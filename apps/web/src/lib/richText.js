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

// SHOW_TEXT | SHOW_ELEMENT, without depending on the globals existing in every DOM.
const SHOW_TEXT = 4;
const SHOW_ELEMENT = 1;

const BLOCK_TAGS = new Set(['h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'p', 'ul', 'ol', 'li', 'blockquote', 'pre', 'hr', 'table']);

/** Render-only fillers never belong in the stored markdown. */
const clean = (s) => s.replace(/[\u200B\uFFFF]/g, '');
/** The ZWSP is dropped while serializing; the caret marker survives it (caretMdOffset). */
const stripZwsp = (s) => s.replace(/\u200B/g, '');
const hasContent = (s) => clean(s).trim() !== '';

/**
 * Trim edges but keep a single leading/trailing line break: that break is the
 * empty line the caret is sitting on after Enter, not noise.
 */
const tidy = (s) => {
  let t = s.trim();
  if (!t) return '';
  if (/^[ \t]*\n/.test(s)) t = `\n${t}`;
  if (/\n[ \t]*$/.test(s)) t = `${t}\n`;
  return t;
};

/* ----------------------------------------------------------- DOM → markdown */

/** Recover the markdown that produced the edited DOM of one block. */
export function domToMd(root) {
  return clean(serializeChildren(root));
}

/** Serialize a container's children as block markdown. */
function serializeChildren(node) {
  const parts = [];
  let inline = '';
  const pushInline = () => {
    const part = tidy(stripZwsp(inline));
    if (hasContent(part)) parts.push(part);
    inline = '';
  };
  for (const child of node.childNodes) {
    if (child.nodeType === 3) {
      if (!child.data.trim()) continue; // inter-block whitespace
      inline += child.data;
      continue;
    }
    if (child.nodeType !== 1) continue;
    const tag = child.tagName.toLowerCase();
    if (BLOCK_TAGS.has(tag)) {
      if (inline.trim()) pushInline();
      inline = '';
      const part = stripZwsp(serializeBlock(child));
      if (hasContent(part)) parts.push(part);
    } else {
      inline += inlineOf(child);
    }
  }
  if (inline.trim()) pushInline();
  return parts.join('\n\n');
}

/** Trim plain spaces but keep line breaks — a marker's space is not noise. */
const trimSpaces = (s) => s.replace(/^[ \t]+/, '').replace(/[ \t]+$/, '');

function serializeBlock(node) {
  const tag = node.tagName.toLowerCase();
  if (tag === 'p') return trimSpaces(inlineOf(node));
  if (tag[0] === 'h' && tag.length === 2 && tag >= 'h1' && tag <= 'h6') {
    return `${'#'.repeat(Number(tag[1]))} ${trimSpaces(inlineOf(node))}`;
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
  return trimSpaces(inlineOf(node));
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

/**
 * The caret coordinate system. Every text node with content contributes its
 * length and every `<br>` contributes one — a line break is a place the caret
 * sits between, and without counting it a restored caret would climb back
 * above the line it was dropped on. Whitespace-only text nodes (the `\n` the
 * html draws between blocks) take no coordinate: they are not a place a caret
 * can meaningfully sit.
 *
 * The walks below traverse childNodes by hand. A TreeWalker filter would say
 * the same, but some DOMs (jsdom) silently ignore filters, and the editor's
 * caret arithmetic must not drift between environments.
 */
const isSeat = (n) => n.nodeType === 3 && (/[^\s]/.test(n.data) || /[\u200B\uFFFF]/.test(n.data));
const isBr = (n) => n.nodeType === 1 && n.tagName?.toLowerCase() === 'br';

/** Sum of caret coordinates inside a subtree. */
function subtreeSize(node) {
  if (node.nodeType === 3) return isSeat(node) ? node.data.length : 0;
  if (!isBr(node)) {
    if (node.nodeType !== 1) return 0;
    let sum = 0;
    for (const child of node.childNodes) sum += subtreeSize(child);
    return sum;
  }
  return 1;
}

/** The caret seats of a container, in document order. */
function seatList(root) {
  const out = [];
  const collect = (node) => {
    for (const child of node.childNodes) {
      if (child.nodeType === 3) {
        if (isSeat(child)) out.push(child);
      } else if (isBr(child)) {
        out.push(child);
      } else if (child.nodeType === 1) {
        collect(child);
      }
    }
  };
  collect(root);
  return out;
}

/** Total visible text length of a container. */
export function textLength(root) {
  return subtreeSize(root);
}

/** The caret as an offset into all the block's visible text. */
export function caretTextOffset(root) {
  const sel = window.getSelection();
  if (!sel || !sel.rangeCount) return textLength(root);
  const range = sel.getRangeAt(0);
  const node = range.startContainer;
  if (!root.contains(node)) return textLength(root);

  // Where the anchor sits in the seat coordinates: a text anchor contributes
  // its own offset; an element anchor contributes the seats of the children
  // before its index.
  const positionOf = (container) => {
    if (container === node) {
      if (node.nodeType === 3) return range.startOffset;
      let sum = 0;
      let child = container.firstChild;
      for (let i = 0; child && i < range.startOffset; i++, child = child.nextSibling) {
        sum += subtreeSize(child);
      }
      return sum;
    }
    let sum = 0;
    for (const child of container.childNodes) {
      if (child.contains(node)) {
        const inner = positionOf(child);
        return inner == null ? null : sum + inner;
      }
      sum += subtreeSize(child);
    }
    return null;
  };
  return positionOf(root) ?? textLength(root);
}

/** Put the caret at a plain-text offset inside the block. */
export function setCaretAtTextOffset(root, offset) {
  const target = Math.max(0, offset | 0);
  const seats = seatList(root);
  let remaining = target;
  for (const seat of seats) {
    const size = seat.nodeType === 3 ? seat.data.length : 1;
    // A boundary position (the very end of one node) belongs to the next
    // node's start: that is the side the probe that produced the offset meant.
    if (seat.nodeType === 3 ? remaining < size : remaining <= size) {
      if (seat.nodeType === 3) placeCaret(seat, remaining);
      else if (remaining === 1) placeCaretBeside(seat, 'after');
      else placeCaretBeside(seat, 'before');
      return;
    }
    remaining -= size;
  }
  // Past the end — caret after the last seat; no seats at all — block start.
  const sel = window.getSelection();
  const range = document.createRange();
  const last = seats[seats.length - 1];
  if (last) {
    if (last.nodeType === 3) range.setStart(last, last.data.length);
    else range.setStartAfter(last); // a trailing <br> — caret after the break
    range.collapse(true);
  } else {
    range.selectNodeContents(root);
    range.collapse(true);
  }
  sel?.removeAllRanges();
  sel?.addRange(range);
}

function placeCaret(node, offset) {
  const sel = window.getSelection();
  const range = document.createRange();
  range.setStart(node, offset);
  range.collapse(true);
  sel?.removeAllRanges();
  sel?.addRange(range);
}

/** The caret just before or just after a `<br>` — the two sides of a break. */
function placeCaretBeside(br, side) {
  const sel = window.getSelection();
  const range = document.createRange();
  if (side === 'after') range.setStartAfter(br);
  else range.setStartBefore(br);
  range.collapse(true);
  sel?.removeAllRanges();
  sel?.addRange(range);
}

/** The caret as an offset into the block's markdown. */
export function caretMdOffset(root) {
  const sel = window.getSelection();
  if (!sel || !sel.rangeCount) return 0;
  if (!root.contains(sel.anchorNode)) {
    // An element-boundary caret (a range set on the wrapper or the surface,
    // as programmatic ranges and boundary clicks produce) still names a real
    // position: re-anchor it at the body's own start/end and fall through to
    // the normal probe mapping so the markdown offset stays exact.
    const atStart = sel.anchorNode === root.parentElement && sel.anchorOffset === 0;
    const r = document.createRange();
    r.selectNodeContents(root);
    r.collapse(atStart);
    sel.removeAllRanges();
    sel.addRange(r);
  }
  const range = sel.getRangeAt(0);
  // Find the marker's seat with the same walk setCaretAtTextOffset uses: a
  // caret anchored to an element boundary (after a list's last item, say) has
  // no text node to hold the marker, and inter-block whitespace is not a seat
  // either — the serialization would drop the marker and lose the position.
  const seat = seatForTextOffset(root, caretTextOffset(root));
  const probe = document.createRange();
  if (seat?.node) probe.setStart(seat.node, seat.offset);
  else if (seat?.br) {
    if (seat.after) probe.setStartAfter(seat.br);
    else probe.setStartBefore(seat.br);
  } else {
    probe.setStart(range.startContainer, range.startOffset);
  }
  probe.collapse(true);
  const marker = document.createTextNode(CARET);
  probe.insertNode(marker);
  const raw = serializeChildren(root);
  const at = raw.indexOf(CARET);
  marker.remove();
  sel.removeAllRanges();
  sel.addRange(range);
  if (at < 0) return 0;
  // The stored markdown drops the invisible fillers; count only real characters.
  return clean(raw.slice(0, at)).length;
}

/** Where a text offset sits: a (text node, offset) or a side of a `<br>`. */
export function seatForTextOffset(root, target) {
  const seats = seatList(root);
  let remaining = Math.max(0, target | 0);
  for (const seat of seats) {
    const size = seat.nodeType === 3 ? seat.data.length : 1;
    // The marker wants the caret's exact spot, so a text node's own end is a
    // valid seat here (unlike setCaretAtTextOffset, whose offsets carry the
    // probe's intent).
    if (remaining <= size) {
      if (seat.nodeType === 3) return { node: seat, offset: remaining };
      return { br: seat, after: remaining === 1 };
    }
    remaining -= size;
  }
  return null;
}

/**
 * Where a given markdown offset lands in the rendered text.
 *
 * The probe renders the markdown with an invisible marker at that offset and
 * reports the marker's own text offset — that is where the caret belongs after
 * the document re-renders (Enter inside a list, indenting, …). The probe adds
 * no width of its own: the marker character is the caret's seat, so sums stay
 * in real-DOM coordinates. A caret asked for inside `# ` or `- ` lands right
 * after the marker — the drawn form has no marker characters to sit within.
 */
export function textOffsetForMd(md, mdOffset) {
  const source = String(md ?? '');
  let offset = Math.max(0, Math.min(mdOffset | 0, source.length));
  const lineStart = source.lastIndexOf('\n', offset - 1) + 1;
  const marker = /^[ \t]*(?:#{1,6} |>(?:[ \t]|$)|(?:[-+*]|\d+[.)])[ \t])/.exec(source.slice(lineStart));
  if (marker && offset < lineStart + marker[0].length) offset = lineStart + marker[0].length;
  const probe = `${source.slice(0, offset)}${CARET}${source.slice(offset)}`;
  const holder = document.createElement('div');
  holder.innerHTML = renderMarkdown(probe);
  let sum = 0;
  for (const seat of seatList(holder)) {
    if (seat.nodeType === 3) {
      const at = seat.data.indexOf(CARET);
      if (at !== -1) return sum + at;
      sum += seat.data.length;
    } else {
      sum += 1; // <br>
    }
  }
  return sum;
}
