import { splitMarkdownBlocks } from '@ai-studio/format';

/**
 * Parse markdown into a small block/run tree that both the .pptx and .docx
 * exporters can walk.
 *
 * This is deliberately not a full markdown implementation — it covers exactly the
 * constructs the editors can produce, and everything else falls through as plain
 * text rather than being dropped.
 */
export function parseMarkdown(md) {
  return splitMarkdownBlocks(md).map(toBlock).filter(Boolean);
}

function toBlock(part) {
  const text = part.md;
  const trimmed = text.trim();

  const heading = trimmed.match(/^(#{1,6})[ \t]+(.*)$/);
  if (heading) {
    return { type: 'heading', level: heading[1].length, runs: inlineRuns(heading[2]) };
  }

  const fence = trimmed.match(/^(?:```|~~~)([^\n]*)\n([\s\S]*?)\n?(?:```|~~~)?$/);
  if (fence) {
    return { type: 'code', lang: fence[1].trim(), text: fence[2] };
  }

  if (/^(?:---+|\*\*\*+|___+)$/.test(trimmed)) return { type: 'hr' };

  const image = trimmed.match(/^!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)$/);
  if (image) return { type: 'image', alt: image[1], src: image[2] };

  if (/^\|/.test(trimmed)) {
    const rows = trimmed
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line.startsWith('|') && !/^\|[\s:|-]+\|?$/.test(line))
      .map((line) =>
        line
          .replace(/^\|/, '')
          .replace(/\|$/, '')
          .split('|')
          .map((cell) => inlineRuns(cell.trim()))
      );
    if (rows.length) return { type: 'table', rows };
  }

  if (/^>/.test(trimmed)) {
    const body = trimmed
      .split('\n')
      .map((line) => line.replace(/^[ \t]*>[ \t]?/, ''))
      .join('\n');
    return { type: 'quote', runs: inlineRuns(body) };
  }

  const listMatch = trimmed.match(/^([-+*]|\d+[.)])[ \t]+/);
  if (listMatch) {
    const ordered = /\d/.test(listMatch[1]);
    const items = [];
    for (const line of trimmed.split('\n')) {
      const m = line.match(/^([ \t]*)(?:[-+*]|\d+[.)])[ \t]+(.*)$/);
      if (m) {
        items.push({ level: Math.floor(m[1].replace(/\t/g, '  ').length / 2), runs: inlineRuns(m[2]) });
      } else if (items.length) {
        // A wrapped continuation line belongs to the previous item.
        items[items.length - 1].runs.push({ text: ` ${line.trim()}` });
      }
    }
    if (items.length) return { type: 'list', ordered, items };
  }

  if (!trimmed) return null;
  return { type: 'paragraph', runs: inlineRuns(text.replace(/\n/g, ' ')) };
}

const INLINE = [
  { re: /^`([^`]+)`/, mark: { code: true } },
  { re: /^\*\*\*([^*]+)\*\*\*/, mark: { bold: true, italic: true } },
  { re: /^\*\*([^*]+)\*\*/, mark: { bold: true } },
  { re: /^__([^_]+)__/, mark: { bold: true } },
  { re: /^~~([^~]+)~~/, mark: { strike: true } },
  { re: /^\*([^*]+)\*/, mark: { italic: true } },
  { re: /^_([^_]+)_/, mark: { italic: true } },
];

/**
 * Split inline markdown into styled runs.
 * Unmatched syntax characters stay as literal text — an exporter should never
 * silently swallow a stray asterisk.
 */
export function inlineRuns(text) {
  const src = String(text ?? '');
  const runs = [];
  let plain = '';
  let i = 0;

  const flush = () => {
    if (plain) {
      runs.push({ text: plain });
      plain = '';
    }
  };

  while (i < src.length) {
    const rest = src.slice(i);

    const link = rest.match(/^\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)/);
    if (link) {
      flush();
      runs.push({ text: link[1] || link[2], link: link[2] });
      i += link[0].length;
      continue;
    }

    const image = rest.match(/^!\[([^\]]*)\]\([^)]*\)/);
    if (image) {
      flush();
      // Inline images become their alt text; block images are handled separately.
      if (image[1]) runs.push({ text: image[1], italic: true });
      i += image[0].length;
      continue;
    }

    let matched = false;
    for (const rule of INLINE) {
      const m = rest.match(rule.re);
      if (!m) continue;
      flush();
      runs.push({ text: m[1], ...rule.mark });
      i += m[0].length;
      matched = true;
      break;
    }
    if (matched) continue;

    plain += src[i];
    i++;
  }
  flush();
  return runs.length ? runs : [{ text: '' }];
}

/** Flatten runs back to plain text, for places that cannot carry formatting. */
export function runsToText(runs) {
  return (runs ?? []).map((r) => r.text).join('');
}

/** Plain text of a whole block tree, one line per block. */
export function blocksToText(blocks) {
  const lines = [];
  for (const block of blocks) {
    switch (block.type) {
      case 'heading':
      case 'paragraph':
      case 'quote':
        lines.push(runsToText(block.runs));
        break;
      case 'list':
        block.items.forEach((item, i) => {
          lines.push(`${block.ordered ? `${i + 1}.` : '•'} ${runsToText(item.runs)}`);
        });
        break;
      case 'code':
        lines.push(block.text);
        break;
      case 'table':
        block.rows.forEach((row) => lines.push(row.map(runsToText).join('\t')));
        break;
      case 'image':
        lines.push(block.alt || '(이미지)');
        break;
      default:
        break;
    }
  }
  return lines.join('\n');
}
