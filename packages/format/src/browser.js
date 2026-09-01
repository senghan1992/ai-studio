/**
 * Browser-safe entry point: everything except `project.js`, which touches
 * `node:fs`. The web editor imports this so the same parsers run on both sides
 * of the wire — a slide edited in the browser serializes byte-identically to
 * what the server would have written.
 */
export * from './ids.js';
export * from './frontmatter.js';
export * from './blocks.js';
export * from './mdblocks.js';
export * from './geometry.js';
export * from './deck.js';
export * from './doc.js';
export * from './grid.js';
export * from './chart.js';
export { buildDigest } from './digest.js';

export const TYPE_LABEL = { deck: '프레젠테이션', doc: '문서', grid: '스프레드시트' };
export const TYPE_EXT = { deck: '.aideck', doc: '.aidoc', grid: '.aigrid' };
