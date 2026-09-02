import React, { useMemo } from 'react';
import { shapePath, isOpenShape, presetRotation, canDraw } from '../lib/shapeSvg.js';

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
  const path = useMemo(() => shapePath(preset, shape) ?? shapePath('rect', null), [preset, shape]);

  const fill = shape?.fill?.color;
  const opacity = shape?.fill?.opacity;
  const line = shape?.line;
  const open = isOpenShape(preset);
  const spin = presetRotation(preset);

  // A stroke straddles the path, so half of it would fall outside the box.
  const stroke = line ? Math.max(0.5, line.width ?? 1) : 0;
  const inset = stroke / 2;

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
          strokeWidth={
            // The stroke is in the block's pixels but the path is in the 0..100
            // box, so it has to be scaled back or a 2px outline draws as 2% of
            // the shape — thick on a small shape, invisible on a large one.
            stroke ? (stroke * 100) / Math.max(width, height, 1) : undefined
          }
          strokeDasharray={dashArray(line)}
          strokeLinejoin="round"
          vectorEffect="non-scaling-stroke"
        />
      </g>
      {inset > 0 ? null : null}
    </svg>
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
