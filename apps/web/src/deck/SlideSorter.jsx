import React, { useState } from 'react';

/**
 * Slide thumbnail strip with drag-to-reorder.
 *
 * Thumbnails are rendered from the real block geometry scaled down, not from a
 * screenshot, so they stay correct without any rendering pipeline.
 */
export default function SlideSorter({
  slides, current, onSelect, onReorder, onAdd, onContextMenu, onDelete, onDuplicate,
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
            <button
              key={slide.id ?? index}
              className="sorter-item"
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
                if (e.key === 'Delete' || e.key === 'Backspace') {
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
              <span
                className={`sorter-item__thumb${dragOver === index && dragFrom !== index ? ' is-drop' : ''}`}
                style={{
                  background: slide.canvas?.bg ?? '#fff',
                  // The deck's own shape, not an assumed 16:9. A 4:3 or portrait
                  // deck stretched into a widescreen thumbnail is the first thing
                  // that looks wrong about it.
                  aspectRatio: `${slide.canvas?.w ?? 1280} / ${slide.canvas?.h ?? 720}`,
                }}
              >
                {slide.blocks.map((block) => (
                  <span
                    key={block.id}
                    className={`thumb-block thumb-block--${block.kind}`}
                    style={{
                      left: `${(block.x / (slide.canvas?.w ?? 1280)) * 100}%`,
                      top: `${(block.y / (slide.canvas?.h ?? 720)) * 100}%`,
                      width: `${(block.w / (slide.canvas?.w ?? 1280)) * 100}%`,
                      height: `${(block.h / (slide.canvas?.h ?? 720)) * 100}%`,
                      textAlign: block.style?.align ?? 'left',
                      fontWeight: (block.style?.weight ?? 400) >= 600 ? 700 : 400,
                    }}
                  >
                    {block.kind === 'text' ? plain(block.md) : ''}
                  </span>
                ))}
              </span>
            </button>
          ))}
        </div>
      </div>
    </aside>
  );
}

function plain(md) {
  const text = String(md ?? '')
    .replace(/[#*`>|]/g, '')
    .replace(/!\[[^\]]*\]\([^)]*\)/g, '')
    .replace(/\s+/g, ' ')
    .trim();
  return text.slice(0, 60);
}
