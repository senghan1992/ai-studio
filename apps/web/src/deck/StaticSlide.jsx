import React from 'react';
import { listVars } from './SlideCanvas.jsx';
import { renderMarkdown } from '../lib/markdown.js';
import { assetUrl, isProjectAsset } from '../api.js';
import ChartView from '../components/ChartView.jsx';
import ShapeView from '../components/ShapeView.jsx';
import TableView from '../components/TableView.jsx';
import { blockTransform, cropImageStyle } from '../lib/imageStyle.js';

/**
 * One slide, rendered read-only at a given scale.
 *
 * The slideshow, the print layout and the sidebar thumbnail draw the same thing:
 * the canvas's block geometry with nothing editable on it. Keeping the renderer
 * in one place is what guarantees the show, the print and the canvas cannot
 * disagree about what a slide looks like.
 *
 * The scale is applied **once**, as a CSS transform on the whole layer, rather
 * than multiplied into every number. A table's column widths, a chart's SVG and
 * its labels, and the type all carry their own internal metrics; scaling the
 * outer geometry while leaving those alone made a thumbnail show only the first
 * cells of a table and clip a chart. One transform shrinks everything together.
 */
export default function StaticSlide({ slide, folder, scale = 1, className, blockClass }) {
  const canvas = slide.canvas ?? { w: 1280, h: 720, bg: '#ffffff' };
  const blocks = [...(slide.blocks ?? [])].sort((a, b) => (a.z ?? 0) - (b.z ?? 0));

  return (
    <div
      className={className}
      style={{
        position: 'relative',
        overflow: 'hidden',
        width: canvas.w * scale,
        height: canvas.h * scale,
        background: canvas.bg ?? '#ffffff',
      }}
    >
      <div
        style={{
          position: 'absolute',
          left: 0,
          top: 0,
          width: canvas.w,
          height: canvas.h,
          transform: `scale(${scale})`,
          transformOrigin: 'top left',
        }}
      >
        {blocks.map((block) => (
          <div
            key={block.id}
            className={blockClass}
            style={{
              position: 'absolute',
              overflow: 'hidden',
              left: block.x,
              top: block.y,
              width: block.w,
              height: block.h,
              zIndex: block.z ?? 1,
              transform: blockTransform(block),
            }}
          >
            {/* A shape is its preset geometry here exactly as on the canvas: a
                diamond drawn as a CSS rectangle is not the slide the author
                built, and a presentation is where that shows. */}
            {block.kind === 'shape' && (
              <ShapeView shape={block.shape} width={block.w} height={block.h} />
            )}
            <BlockContent block={block} folder={folder} canvasBg={canvas.bg} />
          </div>
        ))}
      </div>
    </div>
  );
}

function BlockContent({ block, folder, canvasBg }) {
  if (block.kind === 'chart') {
    return (
      <ChartView
        md={block.md}
        width={block.w}
        height={block.h}
        surface={canvasBg ?? '#ffffff'}
      />
    );
  }

  // A table keeps its column widths, merges and header band — the same renderer
  // the canvas uses, read-only because there is nothing to edit here.
  if (block.kind === 'table') {
    return <TableView md={block.md} spec={block.table} width={block.w} height={block.h} />;
  }

  if (block.kind === 'image') {
    const match = String(block.md ?? '').match(/!\[([^\]]*)\]\(([^)\s]+)/);
    if (!match) return null;
    const src = isProjectAsset(match[2]) ? assetUrl(folder, match[2]) : match[2];
    const radius = block.style?.radius ?? 0;
    // A cropped image is enlarged and offset; the block wrapper's overflow
    // clips it. Otherwise it is contained (or covered) in the box as before.
    const crop = cropImageStyle(block.style?.crop);
    const imgStyle = crop
      ? { ...crop, borderRadius: radius }
      : {
          width: '100%',
          height: '100%',
          objectFit: block.style?.fit === 'cover' ? 'cover' : 'contain',
          borderRadius: radius,
        };
    return <img src={src} alt={match[1]} style={imgStyle} />;
  }

  const style = block.style ?? {};
  return (
    <div
      className="md"
      style={{
        // The layer is transformed, so the type is its natural size here and
        // shrinks with everything else.
        fontSize: `${style.fontSize ?? 20}px`,
        fontWeight: style.weight,
        textAlign: style.align,
        color: style.color,
        lineHeight: style.lineHeight ?? 1.45,
        '--space-before': style.spaceBefore != null ? `${style.spaceBefore}px` : undefined,
        '--space-after': style.spaceAfter != null ? `${style.spaceAfter}px` : undefined,
        ...listVars(style.list),
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        justifyContent:
          style.valign === 'middle' ? 'center' : style.valign === 'bottom' ? 'flex-end' : 'flex-start',
        padding: '4px 6px',
      }}
      dangerouslySetInnerHTML={{
        __html: renderMarkdown(block.md, {
          assetResolver: (src) => (isProjectAsset(src) ? assetUrl(folder, src) : src),
        }),
      }}
    />
  );
}