import React, { useCallback, useEffect, useState } from 'react';
import { renderMarkdown } from '../lib/markdown.js';
import { assetUrl, isProjectAsset } from '../api.js';
import ChartView from '../components/ChartView.jsx';

/**
 * Full-screen presentation mode.
 *
 * PowerPoint's F5 is muscle memory, and its absence is the loudest missing thing
 * in a deck editor. Same block geometry as the canvas, scaled to the viewport, with
 * arrow/space/PageDown advancing, Esc leaving, and `S` showing speaker notes.
 */
export default function Slideshow({ slides, start = 0, folder, onClose }) {
  const [index, setIndex] = useState(start);
  const [showNotes, setShowNotes] = useState(false);
  const [viewport, setViewport] = useState({ w: window.innerWidth, h: window.innerHeight });

  const slide = slides[Math.min(index, slides.length - 1)];
  const canvas = slide?.canvas ?? { w: 1280, h: 720, bg: '#ffffff' };

  const go = useCallback(
    (delta) => setIndex((i) => Math.min(slides.length - 1, Math.max(0, i + delta))),
    [slides.length]
  );

  useEffect(() => {
    const onKey = (e) => {
      switch (e.key) {
        case 'Escape':
          e.preventDefault();
          onClose();
          break;
        case 'ArrowRight':
        case 'ArrowDown':
        case 'PageDown':
        case ' ':
        case 'Enter':
          e.preventDefault();
          go(1);
          break;
        case 'ArrowLeft':
        case 'ArrowUp':
        case 'PageUp':
        case 'Backspace':
          e.preventDefault();
          go(-1);
          break;
        case 'Home':
          e.preventDefault();
          setIndex(0);
          break;
        case 'End':
          e.preventDefault();
          setIndex(slides.length - 1);
          break;
        case 's':
        case 'S':
          e.preventDefault();
          setShowNotes((v) => !v);
          break;
        default:
          break;
      }
    };
    const onResize = () => setViewport({ w: window.innerWidth, h: window.innerHeight });

    window.addEventListener('keydown', onKey);
    window.addEventListener('resize', onResize);
    // Try real fullscreen, but keep working if the browser refuses.
    document.documentElement.requestFullscreen?.().catch(() => {});
    return () => {
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('resize', onResize);
      if (document.fullscreenElement) document.exitFullscreen?.().catch(() => {});
    };
  }, [go, onClose, slides.length]);

  if (!slide) return null;

  const scale = Math.min((viewport.w - 40) / canvas.w, (viewport.h - 40) / canvas.h);
  const blocks = [...slide.blocks].sort((a, b) => (a.z ?? 0) - (b.z ?? 0));

  return (
    <div className="slideshow" onClick={(e) => e.target === e.currentTarget && go(1)}>
      <div
        className="slideshow__stage"
        style={{
          width: canvas.w * scale,
          height: canvas.h * scale,
          background: canvas.bg ?? '#ffffff',
        }}
      >
        {blocks.map((block) => (
          <div
            key={block.id}
            className="slideshow__block"
            style={{
              left: block.x * scale,
              top: block.y * scale,
              width: block.w * scale,
              height: block.h * scale,
              zIndex: block.z ?? 1,
              ...(block.kind === 'shape'
                ? { background: block.style?.fill ?? '#e5e7eb', borderRadius: (block.style?.radius ?? 6) * scale }
                : {}),
            }}
          >
            <BlockContent block={block} scale={scale} folder={folder} canvasBg={canvas.bg} />
          </div>
        ))}
      </div>

      {showNotes && slide.notes?.trim() && <div className="slideshow__notes">{slide.notes}</div>}

      <div className="slideshow__bar" onClick={(e) => e.stopPropagation()}>
        <button type="button" onClick={() => go(-1)} disabled={index === 0}>
          ← 이전
        </button>
        <span className="slideshow__count">
          {index + 1} / {slides.length}
        </span>
        <button type="button" onClick={() => go(1)} disabled={index >= slides.length - 1}>
          다음 →
        </button>
        <button type="button" onClick={() => setShowNotes((v) => !v)} title="S">
          {showNotes ? '노트 숨기기' : '발표자 노트'}
        </button>
        <button type="button" onClick={onClose} title="Esc">
          끝내기
        </button>
      </div>
    </div>
  );
}

function BlockContent({ block, scale, folder, canvasBg }) {
  if (block.kind === 'chart') {
    return <ChartView md={block.md} width={block.w * scale} height={block.h * scale} surface={canvasBg ?? '#ffffff'} />;
  }

  if (block.kind === 'image') {
    const match = String(block.md ?? '').match(/!\[([^\]]*)\]\(([^)\s]+)/);
    if (!match) return null;
    const src = isProjectAsset(match[2]) ? assetUrl(folder, match[2]) : match[2];
    return (
      <img
        src={src}
        alt={match[1]}
        style={{
          width: '100%',
          height: '100%',
          objectFit: block.style?.fit === 'cover' ? 'cover' : 'contain',
          borderRadius: (block.style?.radius ?? 0) * scale,
        }}
      />
    );
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
