import React, { useCallback, useEffect, useState } from 'react';
import StaticSlide from './StaticSlide.jsx';

/**
 * Full-screen presentation mode.
 *
 * PowerPoint's F5 is muscle memory, and its absence is the loudest missing thing
 * in a deck editor. Same block geometry as the canvas, scaled to the viewport, with
 * arrow/space/PageDown advancing, Esc leaving, `S` showing speaker notes, and
 * `B`/`W` blanking the screen the way PowerPoint's presenter does.
 */
export default function Slideshow({ slides, start = 0, folder, onClose }) {
  const [index, setIndex] = useState(start);
  const [showNotes, setShowNotes] = useState(false);
  /** `null`, `'black'` or `'white'` — PowerPoint's B and W keys. */
  const [blank, setBlank] = useState(null);
  const [viewport, setViewport] = useState({ w: window.innerWidth, h: window.innerHeight });

  const slide = slides[Math.min(index, slides.length - 1)];
  const canvas = slide?.canvas ?? { w: 1280, h: 720, bg: '#ffffff' };

  const go = useCallback((delta) => {
    // Advancing un-blanks the screen, as it does in PowerPoint.
    setBlank(null);
    setIndex((i) => Math.min(slides.length - 1, Math.max(0, i + delta)));
  }, [slides.length]);

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
        // PowerPoint blanks the screen on B (black) and W (white), and any
        // other key brings the slide back. Presenters use it to take the room's
        // attention off the screen.
        case 'b':
        case 'B':
        case '.':
          e.preventDefault();
          setBlank((v) => (v === 'black' ? null : 'black'));
          break;
        case 'w':
        case 'W':
        case ',':
          e.preventDefault();
          setBlank((v) => (v === 'white' ? null : 'white'));
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

  return (
    <div className="slideshow" onClick={(e) => e.target === e.currentTarget && go(1)}>
      {/* The same read-only renderer the print layout uses — the show and the
          paper cannot disagree about what a slide looks like. */}
      <StaticSlide
        slide={slide}
        folder={folder}
        scale={scale}
        className="slideshow__stage"
        blockClass="slideshow__block"
      />

      {/* B and W blank the screen over everything, including the notes. */}
      {blank && (
        <div
          className="slideshow__blank"
          style={{ background: blank === 'white' ? '#ffffff' : '#000000' }}
          onClick={() => setBlank(null)}
        />
      )}

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
