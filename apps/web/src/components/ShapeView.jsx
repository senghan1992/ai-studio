import React, { useId, useMemo } from 'react';
import { shapePath, isOpenShape, presetRotation, markerPath, canDraw } from '../lib/shapeSvg.js';

/**
 * A shape, drawn from its preset geometry.
 *
 * The path is defined in a 0..100 square and stretched to the block's box with
 * `preserveAspectRatio="none"`, which is how PowerPoint treats a preset: the
 * geometry belongs to the bounding box, not to a fixed aspect ratio.
 *
 * A preset this renderer has no path for draws as its bounding rectangle. That
 * is the honest fallback — the stored name is never changed, so exporting gives
 * PowerPoint back the shape it asked for.
 */
export default function ShapeView({ shape, width, height }) {
  const preset = shape?.preset ?? 'rect';
  // The box matters: corner treatments convert against its short side, so the
  // path has to be recomputed when the block is resized.
  const path = useMemo(
    () => shapePath(preset, shape, { w: width, h: height }) ?? shapePath('rect', null),
    [preset, shape, width, height]
  );
  const uid = useId().replace(/[^a-zA-Z0-9_-]/g, '');

  const fill = shape?.fill?.color;
  const opacity = shape?.fill?.opacity;
  const line = shape?.line;
  const open = isOpenShape(preset);
  const spin = presetRotation(preset);

  // The stroke is in screen pixels: `vectorEffect="non-scaling-stroke"` keeps
  // it out of the 0..100 box mapping, so the width crosses over unchanged.
  // Scaling it back by the box size (as was done before) drew every imported
  // outline hairline-thin — a 2px border on a 600px shape came out 0.33px.
  const stroke = line ? Math.max(0.5, line.width ?? 1) : 0;
  const head = markerPath(line?.head);
  const tail = markerPath(line?.tail);

  return (
    <svg
      className="shape"
      viewBox="0 0 100 100"
      width={width}
      height={height}
      preserveAspectRatio="none"
      aria-hidden="true"
      focusable="false"
    >
      {(head || tail) && (
        <defs>
          {head && <EndMarker id={`${uid}h`} d={head} color={line.color} orient="auto" />}
          {tail && <EndMarker id={`${uid}t`} d={tail} color={line.color} orient="auto-start-reverse" />}
        </defs>
      )}
      <g
        transform={
          spin
            ? `rotate(${spin} 50 50)`
            : undefined
        }
      >
        <path
          d={path}
          fill={open || !fill ? 'none' : fill}
          fillOpacity={opacity !== undefined && opacity < 100 ? opacity / 100 : undefined}
          fillRule="evenodd"
          stroke={line?.color ?? 'none'}
          strokeWidth={stroke ? stroke : undefined}
          strokeDasharray={dashArray(line)}
          strokeLinejoin="round"
          vectorEffect="non-scaling-stroke"
          markerStart={tail ? `url(#${uid}t)` : undefined}
          markerEnd={head ? `url(#${uid}h)` : undefined}
        />
      </g>
    </svg>
  );
}

/**
 * One line-end marker. `markerUnits` defaults to `strokeWidth`, so the head
 * scales with the line's own width the way Office draws it.
 */
function EndMarker({ id, d, color, orient }) {
  const open = d.charAt(0) === 'M' && !d.endsWith('Z');
  return (
    <marker
      id={id}
      viewBox="0 0 10 10"
      refX="8"
      refY="5"
      markerWidth="4"
      markerHeight="4"
      orient={orient}
    >
      <path
        d={d}
        fill={open ? 'none' : color}
        stroke={open ? color : 'none'}
        strokeWidth={open ? 1.6 : undefined}
      />
    </marker>
  );
}

/** The dash pattern for a line style, matching the Rust `Dash::svg_dasharray`. */
function dashArray(line) {
  if (!line || !line.dash || line.dash === 'solid') return undefined;
  const w = Math.max(0.5, line.width ?? 1);
  switch (line.dash) {
    case 'dot':
      return `${w} ${w * 2}`;
    case 'dash':
      return `${w * 4} ${w * 3}`;
    case 'dashDot':
      return `${w * 4} ${w * 3} ${w} ${w * 3}`;
    case 'longDash':
      return `${w * 8} ${w * 3}`;
    default:
      return undefined;
  }
}

/** True when the editor can draw this preset rather than a stand-in box. */
export { canDraw };
