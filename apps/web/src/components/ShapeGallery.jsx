import React, { useMemo, useState } from 'react';
import { shapeGallery } from '../core/index.js';
import { shapePath, presetRotation, isOpenShape } from '../lib/shapeSvg.js';

/**
 * Office's shape gallery: the same drawers, the same Korean names, the same
 * order. Clicking a shape inserts it, which is what the gallery does in
 * PowerPoint when nothing is selected.
 *
 * The list comes from the Rust core, so the picker and the file format cannot
 * disagree about which shapes exist.
 */
export default function ShapeGallery({ onPick, onClose, recent = [] }) {
  const groups = useMemo(() => shapeGallery(), []);
  const [query, setQuery] = useState('');

  const needle = query.trim().toLowerCase();
  const filtered = needle
    ? groups
        .map(({ label, presets }) => ({
          label,
          presets: presets.filter(
            (p) => p.label.toLowerCase().includes(needle) || p.name.toLowerCase().includes(needle)
          ),
        }))
        .filter((g) => g.presets.length > 0)
    : groups;

  return (
    <div className="gallery" role="dialog" aria-label="도형">
      <div className="gallery__head">
        <input
          className="gallery__search"
          type="search"
          placeholder="도형 검색"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          autoFocus
        />
        <button className="gallery__close" onClick={onClose} aria-label="닫기">
          ✕
        </button>
      </div>

      <div className="gallery__body">
        {!needle && recent.length > 0 && (
          <Drawer
            label="최근 사용한 도형"
            presets={recent}
            onPick={onPick}
          />
        )}
        {filtered.map(({ label, presets }) => (
          <Drawer key={label} label={label} presets={presets} onPick={onPick} />
        ))}
        {filtered.length === 0 && <p className="gallery__empty">찾는 도형이 없습니다.</p>}
      </div>
    </div>
  );
}

function Drawer({ label, presets, onPick }) {
  return (
    <section className="gallery__group">
      <h3>{label}</h3>
      <div className="gallery__grid">
        {presets.map((preset) => (
          <button
            key={preset.name}
            className="gallery__item"
            title={`${preset.label} (${preset.name})`}
            onClick={() => onPick(preset)}
          >
            <ShapeIcon preset={preset.name} />
            <span className="gallery__label">{preset.label}</span>
          </button>
        ))}
      </div>
    </section>
  );
}

/** A shape at thumbnail size, drawn by the same renderer as the canvas. */
function ShapeIcon({ preset }) {
  const path = shapePath(preset, null);
  const spin = presetRotation(preset);
  const open = isOpenShape(preset);
  return (
    <svg viewBox="-6 -6 112 112" width="26" height="26" aria-hidden="true" focusable="false">
      <g transform={spin ? `rotate(${spin} 50 50)` : undefined}>
        <path
          d={path ?? 'M0,0L100,0L100,100L0,100Z'}
          fill={open ? 'none' : 'currentColor'}
          fillOpacity={open ? undefined : 0.18}
          fillRule="evenodd"
          stroke="currentColor"
          strokeWidth="6"
          strokeLinejoin="round"
          vectorEffect="non-scaling-stroke"
        />
      </g>
    </svg>
  );
}
