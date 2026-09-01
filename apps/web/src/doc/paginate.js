export const PAGE_BREAK = '<!-- page-break -->';

/** True for the block that represents an explicit page break. */
export function isPageBreak(block) {
  return String(block?.md ?? '').trim() === PAGE_BREAK;
}

/**
 * Distribute blocks across pages using measured heights.
 *
 * Word's rule, near enough: a paragraph moves to the next page rather than being
 * split across the boundary, an explicit break always starts a new page, and a
 * block taller than the page gets a page of its own instead of vanishing.
 *
 * @param {{id: string, md: string}[]} blocks
 * @param {Record<string, number>} heights measured px per block id
 * @param {number} contentHeight usable height of one page in px
 * @returns {{id: string}[][]} blocks grouped per page
 */
export function paginate(blocks, heights, contentHeight) {
  const limit = Math.max(80, contentHeight);
  const pages = [[]];
  let used = 0;

  for (const block of blocks) {
    if (isPageBreak(block)) {
      // The break itself is not content; it just closes the page.
      if (pages[pages.length - 1].length > 0 || pages.length === 1) {
        pages.push([]);
        used = 0;
      }
      continue;
    }

    const height = heights[block.id];
    // Before a block has been measured, assume it fits: the second pass corrects it.
    const needed = Number.isFinite(height) ? height : 0;

    const page = pages[pages.length - 1];
    const fits = used + needed <= limit || page.length === 0;
    if (!fits) {
      pages.push([block]);
      used = needed;
      continue;
    }
    page.push(block);
    used += needed;
  }

  // A trailing empty page (from a break at the very end) is still a real page in
  // Word, but an empty *only* page list is not useful.
  return pages.length > 1 && pages[pages.length - 1].length === 0 && blocks.length
    ? pages.slice(0, -1).concat([[]])
    : pages;
}

/** Usable height inside a page after its margins. */
export function contentHeightOf(pageSize, margin) {
  return pageSize.h - (margin?.top ?? 72) - (margin?.bottom ?? 72);
}

/** Usable width inside a page after its margins. */
export function contentWidthOf(pageSize, margin) {
  return pageSize.w - (margin?.left ?? 72) - (margin?.right ?? 72);
}
