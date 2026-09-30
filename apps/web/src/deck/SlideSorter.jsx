import React, { useEffect, useRef, useState } from 'react';
import StaticSlide from './StaticSlide.jsx';

/**
 * The width the thumbnail strip lays out to: `.panel--left` (208px) minus the
 * `.slidesorter` padding (10px a side) and the row's number column and gap
 * (16px + 8px). Used until the browser measures the real box, and where no
 * ResizeObserver exists (the jsdom smoke test), so a thumbnail always draws.
 */
const THUMB_WIDTH = 164;

/**
 * A slide drawn as a real miniature.
 *
 * It is the same `StaticSlide` the slideshow, the print layout and the canvas
 * use, scaled down — so a picture is the picture, a chart is its chart, a card
 * is the shape it was built from. The old strip drew text-only spans and left
 * every other kind an empty grey box, which is why an imported deck's strip
 * looked broken: most of a real deck is shapes and pictures.
 *
 * `React.memo` keeps the other slides from re-rendering while one is edited,
 * and the box is measured rather than assumed so a resized panel stays crisp.
 */
const SlideThumb = React.memo(function SlideThumb({ slide, folder }) {
  const ref = useRef(null);
  const [width, setWidth] = useState(THUMB_WIDTH);

  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === 'undefined') return undefined;
    // `setWidth` bails out on an equal value, so measuring on every resize
    // frame costs nothing until the box actually changes.
    const measure = () => setWidth(Math.round(el.clientWidth) || THUMB_WIDTH);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const canvas = slide.canvas ?? { w: 1280, h: 720, bg: '#ffffff' };
  const cw = canvas.w || 1280;
  return (
    <div
      ref={ref}
      className="sorter-item__thumb"
      style={{
        background: canvas.bg ?? '#fff',
        // The deck's own shape, not an assumed 16:9. A 4:3 or portrait deck
        // stretched into a widescreen thumbnail is the first thing that looks
        // wrong about it.
        aspectRatio: `${canvas.w ?? 1280} / ${canvas.h ?? 720}`,
      }}
    >
      <StaticSlide
        slide={slide}
        folder={folder}
        scale={width / cw}
        className="sorter-item__slide"
      />
    </div>
  );
});

/**
 * Slide thumbnail strip with drag-to-reorder.
 *
 * A thumbnail is a real, read-only render of the slide at thumbnail scale.
 */
export default function SlideSorter({
  slides, current, folder, onSelect, onReorder, onAdd, onContextMenu, onDelete, onDuplicate,
}) {
  const [dragFrom, setDragFrom] = useState(null);
  const [dragOver, setDragOver] = useState(null);

  return (
    <aside className="panel panel--left">
      <div className="panel__head">
        <span>슬라이드</span>
        <button
          className="sheettab__add"
          onClick={onAdd}
          title="새 슬라이드 추가"
          aria-label="새 슬라이드 추가"
        >
          +
        </button>
      </div>
      <div className="panel__body">
        <div className="slidesorter">
          {slides.map((slide, index) => (
            <div
              key={slide.id ?? index}
              className="sorter-item"
              role="button"
              tabIndex={0}
              aria-current={index === current}
              onClick={() => onSelect(index)}
              draggable
              onDragStart={() => setDragFrom(index)}
              onDragOver={(e) => {
                e.preventDefault();
                setDragOver(index);
              }}
              onDragLeave={() => setDragOver((v) => (v === index ? null : v))}
              onDrop={(e) => {
                e.preventDefault();
                if (dragFrom !== null && dragFrom !== index) onReorder(dragFrom, index);
                setDragFrom(null);
                setDragOver(null);
              }}
              onDragEnd={() => {
                setDragFrom(null);
                setDragOver(null);
              }}
              onContextMenu={(e) => onContextMenu?.(e, index)}
              /*
               * The thumbnail strip is where a slide gets deleted or copied in
               * PowerPoint, and Delete is the key people use. Ctrl+D duplicates,
               * and the arrows walk the deck without reaching for the mouse.
               */
              onKeyDown={(e) => {
                const mod = e.metaKey || e.ctrlKey;
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  onSelect(index);
                } else if (e.key === 'Delete' || e.key === 'Backspace') {
                  e.preventDefault();
                  onDelete?.(index);
                } else if (mod && e.key.toLowerCase() === 'd') {
                  e.preventDefault();
                  onDuplicate?.(index);
                } else if (e.key === 'ArrowDown' && index < slides.length - 1) {
                  e.preventDefault();
                  if (mod) onReorder(index, index + 1);
                  else onSelect(index + 1);
                } else if (e.key === 'ArrowUp' && index > 0) {
                  e.preventDefault();
                  if (mod) onReorder(index, index - 1);
                  else onSelect(index - 1);
                }
              }}
              title={`${slide.title} — 끌어서 순서 변경, 우클릭으로 메뉴, Delete로 삭제`}
            >
              <span className="sorter-item__no">{index + 1}</span>
              <SlideThumb slide={slide} folder={folder} />
            </div>
          ))}
        </div>
      </div>
    </aside>
  );
}