const FENCE_OPEN = /^[ \t]*(`{3,}|~{3,})/;

/**
 * Segment plain markdown into top-level block elements.
 *
 * A flow document (Doc) needs paragraph-level blocks for the editor, but its
 * markdown is deliberately marker-free. So we recover block boundaries the way a
 * markdown parser would: blank lines separate, and runs of list items / table
 * rows / quote lines stay together as one logical block.
 */
export function splitMarkdownBlocks(md) {
  const lines = String(md ?? '').split(/\r?\n/);
  const out = [];
  let buf = [];
  let fence = null;
  let run = null; // 'list' | 'table' | 'quote' | null

  const flush = () => {
    const text = buf.join('\n').replace(/\s+$/, '');
    if (text.trim()) out.push({ md: text, type: run ?? classify(text) });
    buf = [];
    run = null;
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    if (fence) {
      buf.push(line);
      if (line.trim().startsWith(fence)) {
        fence = null;
        flush();
      }
      continue;
    }

    const f = line.match(FENCE_OPEN);
    if (f) {
      flush();
      fence = f[1][0].repeat(3);
      buf.push(line);
      continue;
    }

    if (!line.trim()) {
      flush();
      continue;
    }

    if (/^#{1,6}[ \t]+/.test(line)) {
      flush();
      buf.push(line);
      flush();
      continue;
    }

    if (/^[ \t]*(?:---+|\*\*\*+|___+)[ \t]*$/.test(line)) {
      flush();
      buf.push(line);
      run = 'hr';
      flush();
      continue;
    }

    const kind = lineRun(line);
    if (kind) {
      // A different run type starting mid-block begins a new block.
      if (run && run !== kind) flush();
      run = kind;
      buf.push(line);
      continue;
    }

    // Indented continuation of a list item belongs to the list.
    if (run === 'list' && /^[ \t]{2,}\S/.test(line)) {
      buf.push(line);
      continue;
    }
    if (run) flush();
    buf.push(line);
  }
  flush();
  return out;
}

function lineRun(line) {
  if (/^[ \t]*(?:[-+*]|\d+[.)])[ \t]+/.test(line)) return 'list';
  if (/^[ \t]*\|/.test(line)) return 'table';
  if (/^[ \t]*>/.test(line)) return 'quote';
  return null;
}

function classify(text) {
  const t = text.trim();
  if (/^#{1,6}[ \t]+/.test(t)) return 'heading';
  if (/^(?:```|~~~)/.test(t)) return 'code';
  if (/^\|/.test(t)) return 'table';
  if (/^>/.test(t)) return 'quote';
  if (/^(?:[-+*]|\d+[.)])[ \t]+/.test(t)) return 'list';
  if (/^!\[[^\]]*\]\([^)]*\)$/.test(t)) return 'image';
  return 'paragraph';
}

/** Heading level, or 0 for non-headings. */
export function headingLevel(md) {
  const m = String(md ?? '').trim().match(/^(#{1,6})[ \t]+/);
  return m ? m[1].length : 0;
}

/** Strip markdown syntax to plain text, for word counts and outlines. */
export function plainText(md) {
  return String(md ?? '')
    .replace(/```[\s\S]*?```/g, ' ')
    .replace(/`([^`]*)`/g, '$1')
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/^[ \t]*#{1,6}[ \t]+/gm, '')
    .replace(/^[ \t]*>[ \t]?/gm, '')
    .replace(/^[ \t]*(?:[-+*]|\d+[.)])[ \t]+/gm, '')
    .replace(/\*\*([^*]*)\*\*/g, '$1')
    .replace(/\*([^*]*)\*/g, '$1')
    .replace(/~~([^~]*)~~/g, '$1')
    .replace(/^[ \t]*\|/gm, '')
    .replace(/\|/g, ' ')
    .replace(/[ \t]+/g, ' ')
    .trim();
}

export function countWords(text) {
  const t = plainText(text);
  if (!t) return 0;
  // Count CJK characters individually; they carry no spaces between words.
  const cjk = (t.match(/[ㄱ-힝一-鿿぀-ヿ]/g) ?? []).length;
  const latin = (t.replace(/[ㄱ-힝一-鿿぀-ヿ]/g, ' ').match(/[A-Za-z0-9''-]+/g) ?? []).length;
  return cjk + latin;
}
