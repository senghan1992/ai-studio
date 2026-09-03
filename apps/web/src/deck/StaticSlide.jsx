import React from 'react';
import { renderMarkdown } from '../lib/markdown.js';
import { assetUrl, isProjectAsset } from '../api.js';
import ChartView from '../components/ChartView.jsx';
import ShapeView from '../components/ShapeView.jsx';
import TableView from '../components/TableView.jsx';
import { blockTransform, cropImageStyle } from '../lib/imageStyle.js';

/**
 * One slide, rendered read-only at a given scale.
 *
 * The slideshow and the print layout draw the same thing: the canvas's block
 * geometry with nothing editable on it. Keeping the renderer in one place is
 * what guarantees the show, the print and the canvas cannot disagree about
 * what a slide looks like.
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
      {blocks.map((block) => (
        <div
          key={block.id}
          className={blockClass}
          style={{
            position: 'absolute',
            overflow: 'hidden',
            left: block.x * scale,
            top: block.y * scale,
            width: block.w * scale,
            height: block.h * scale,
            zIndex: block.z ?? 1,
            transform: blockTransform(block),
          }}
        >
          {/* A shape is its preset geometry here exactly as on the canvas: a
              diamond drawn as a CSS rectangle is not the slide the author
              built, and a presentation is where that shows. */}
          {block.kind === 'shape' && (
            <ShapeView shape={block.shape} width={block.w * scale} height={block.h * scale} />
          )}
          <BlockContent block={block} scale={scale} folder={folder} canvasBg={canvas.bg} />
        </div>
      ))}
    </div>
  );
}

function BlockContent({ block, scale, folder, canvasBg }) {
  if (block.kind === 'chart') {
    return (
      <ChartView
        md={block.md}
        width={block.w * scale}
        height={block.h * scale}
        surface={canvasBg ?? '#ffffff'}
      />
    );
  }

  // A table keeps its column widths, merges and header band — the same renderer
  // the canvas uses, read-only because there is nothing to edit here.
  if (block.kind === 'table') {
    return (
      <TableView md={block.md} spec={block.table} width={block.w * scale} height={block.h * scale} />
    );
  }

  if (block.kind === 'image') {
    const match = String(block.md ?? '').match(/!\[([^\]]*)\]\(([^)\s]+)/);
    if (!match) return null;
    const src = isProjectAsset(match[2]) ? assetUrl(folder, match[2]) : match[2];
    const radius = (block.style?.radius ?? 0) * scale;
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
        // Scale the type with the stage so the slide looks the same, just bigger.
        fontSize: `${(style.fontSize ?? 20) * scale}px`,
        fontWeight: style.weight,
        textAlign: style.align,
        color: style.color,
        lineHeight: style.lineHeight ?? 1.45,
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        justifyContent:
          style.valign === 'middle' ? 'center' : style.valign === 'bottom' ? 'flex-end' : 'flex-start',
        padding: `${4 * scale}px ${6 * scale}px`,
      }}
      dangerouslySetInnerHTML={{
        __html: renderMarkdown(block.md, {
          assetResolver: (src) => (isProjectAsset(src) ? assetUrl(folder, src) : src),
        }),
      }}
    />
  );
}

