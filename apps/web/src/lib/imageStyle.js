/**
 * Rotation, flip and crop for a block — kept in one place so the canvas, the
 * slideshow and the print layout cannot disagree about what a block looks like.
 *
 * Shapes carry rotation/flip on `block.shape`; images carry the same keys on
 * `block.style`, since an image has no ShapeSpec. Both read through here.
 */

/** The CSS transform for a block's rotation and flips, or undefined for none. */
export function blockTransform(block) {
  const g = block.shape ?? block.style ?? {};
  const parts = [];
  if (g.rotation) parts.push(`rotate(${g.rotation}deg)`);
  if (g.flipH) parts.push('scaleX(-1)');
  if (g.flipV) parts.push('scaleY(-1)');
  return parts.length ? parts.join(' ') : undefined;
}

/**
 * The `<img>` style for a cropped image, or null when nothing is cropped.
 *
 * `crop` is `{ l, t, r, b }` in percent — the fraction trimmed off each edge of
 * the source, exactly PowerPoint's `srcRect`. The image is enlarged so its
 * visible region fills the block and offset so the trimmed edges fall outside;
 * the block wrapper already clips with `overflow: hidden`. `objectFit: 'fill'`
 * stretches the cropped region to the box, matching PowerPoint's `fillRect`.
 */
export function cropImageStyle(crop) {
  if (!crop) return null;
  const l = crop.l || 0;
  const t = crop.t || 0;
  const r = crop.r || 0;
  const b = crop.b || 0;
  if (!(l || t || r || b)) return null;
  const visW = Math.max(0.001, (100 - l - r) / 100);
  const visH = Math.max(0.001, (100 - t - b) / 100);
  return {
    position: 'absolute',
    width: `${100 / visW}%`,
    height: `${100 / visH}%`,
    left: `${-l / visW}%`,
    top: `${-t / visH}%`,
    objectFit: 'fill',
  };
}
