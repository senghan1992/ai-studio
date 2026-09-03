import React from 'react';
import StaticSlide from './StaticSlide.jsx';

/** A4 printable area at the browser's 96dpi, with a 10mm margin all round. */
const PAGE = { landscape: { w: 1047, h: 718 }, portrait: { w: 718, h: 1047 } };

/**
 * The whole deck laid out one slide per page, visible only when printing.
 *
 * PowerPoint's Ctrl+P prints slides, and "PDF로 저장" in the browser's print
 * dialog is how this app produces a PDF. The orientation follows the deck: a
 * widescreen or 4:3 deck prints landscape, an A4-portrait handout prints
 * portrait, and every slide is scaled to fit the page without cropping.
 */
export default function PrintDeck({ slides, folder }) {
  const first = slides?.[0]?.canvas ?? { w: 1280, h: 720 };
  const orientation = first.w >= first.h ? 'landscape' : 'portrait';
  const page = PAGE[orientation];

  return (
    <div className="printdeck" aria-hidden="true">
      <style>{`@media print { @page { size: A4 ${orientation}; margin: 10mm; } }`}</style>
      {(slides ?? []).map((slide) => {
        const canvas = slide.canvas ?? { w: 1280, h: 720 };
        const scale = Math.min(page.w / canvas.w, page.h / canvas.h);
        return (
          <div key={slide.id} className="printdeck__page" style={{ width: page.w, height: page.h }}>
            <StaticSlide
              slide={slide}
              folder={folder}
              scale={scale}
              className="printdeck__slide"
              blockClass="printdeck__block"
            />
          </div>
        );
      })}
    </div>
  );
}
