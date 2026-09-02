/**
 * Drawing a shape: an OOXML `prstGeom` preset name in, an SVG path out.
 *
 * The preset vocabulary is Office's own (see `crates/ai-format/src/shape.rs`),
 * so a `.pptx` opened here draws the shape it says it is. This file is the one
 * place that has to *know* what each name looks like.
 *
 * Paths are written in a 0..100 square and stretched with `preserveAspectRatio`
 * off, which is exactly how PowerPoint treats a preset: the geometry is defined
 * on the shape's bounding box, not at a fixed aspect ratio.
 *
 * A preset with no path here draws as its bounding rectangle — the same
 * fallback a viewer without the geometry has to use — and the name is never
 * changed, so re-exporting gives PowerPoint back the real shape.
 */

/** An adjust handle, as a 0..1 fraction with a default. */
const adj = (shape, name, fallback) => {
  const raw = shape?.adjust?.[name];
  return Number.isFinite(raw) ? raw / 100000 : fallback;
};

const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);

/** A regular polygon inscribed in the box, first vertex at the top. */
function polygon(sides, rotate = -90) {
  const points = [];
  for (let i = 0; i < sides; i++) {
    const angle = ((360 / sides) * i + rotate) * (Math.PI / 180);
    points.push(`${50 + 50 * Math.cos(angle)},${50 + 50 * Math.sin(angle)}`);
  }
  return `M${points.join('L')}Z`;
}

/** A star with `points` tips and the given inner radius fraction. */
function star(points, inner) {
  const path = [];
  for (let i = 0; i < points * 2; i++) {
    const radius = i % 2 === 0 ? 50 : 50 * inner;
    const angle = ((180 / points) * i - 90) * (Math.PI / 180);
    path.push(`${50 + radius * Math.cos(angle)},${50 + radius * Math.sin(angle)}`);
  }
  return `M${path.join('L')}Z`;
}

/** A rectangle with per-corner radii, as an SVG path. */
function rounded([tl, tr, br, bl]) {
  return [
    `M${tl},0`,
    `L${100 - tr},0`,
    tr ? `A${tr},${tr} 0 0 1 100,${tr}` : '',
    `L100,${100 - br}`,
    br ? `A${br},${br} 0 0 1 ${100 - br},100` : '',
    `L${bl},100`,
    bl ? `A${bl},${bl} 0 0 1 0,${100 - bl}` : '',
    `L0,${tl}`,
    tl ? `A${tl},${tl} 0 0 1 ${tl},0` : '',
    'Z',
  ]
    .filter(Boolean)
    .join(' ');
}

/** The same with corners cut off instead of rounded. */
function snipped([tl, tr, br, bl]) {
  return [
    `M${tl},0`,
    `L${100 - tr},0`,
    tr ? `L100,${tr}` : '',
    `L100,${100 - br}`,
    br ? `L${100 - br},100` : '',
    `L${bl},100`,
    bl ? `L0,${100 - bl}` : '',
    `L0,${tl}`,
    'Z',
  ]
    .filter(Boolean)
    .join(' ');
}

/** A block arrow pointing right, in the 0..100 box. */
function rightArrow(shape) {
  // `adj1` is the shaft's half-thickness, `adj2` the head's length.
  const shaft = clamp(adj(shape, 'adj1', 0.5), 0.05, 1) * 50;
  const head = clamp(adj(shape, 'adj2', 0.5), 0.05, 1) * 100;
  const x = 100 - head;
  return `M0,${50 - shaft}L${x},${50 - shaft}L${x},0L100,50L${x},100L${x},${50 + shaft}L0,${50 + shaft}Z`;
}

/** A double-headed arrow along one axis. */
function doubleArrow(shape) {
  const shaft = clamp(adj(shape, 'adj1', 0.5), 0.05, 1) * 50;
  const head = clamp(adj(shape, 'adj2', 0.5), 0.05, 0.5) * 100;
  return [
    `M0,50L${head},0L${head},${50 - shaft}`,
    `L${100 - head},${50 - shaft}L${100 - head},0L100,50`,
    `L${100 - head},100L${100 - head},${50 + shaft}`,
    `L${head},${50 + shaft}L${head},100Z`,
  ].join('');
}

/** A callout: a rounded box with a tail pointing at the adjust handle. */
function callout(shape, radius) {
  const x = 50 + clamp(adj(shape, 'adj1', -0.2), -1.5, 1.5) * 100;
  const y = 50 + clamp(adj(shape, 'adj2', 0.8), -1.5, 1.5) * 100;
  const box = radius ? rounded([radius, radius, radius, radius]) : rounded([0, 0, 0, 0]);
  // The tail leaves the bottom edge, which is where Office puts it by default.
  return `${box} M35,100L${x},${y}L55,100Z`;
}

/** An annulus, used for the donut and the "no smoking" sign. */
function annulus(inner) {
  const r = 50 * (1 - inner);
  return [
    'M50,0A50,50 0 1 1 50,100A50,50 0 1 1 50,0Z',
    `M50,${50 - r}A${r},${r} 0 1 0 50,${50 + r}A${r},${r} 0 1 0 50,${50 - r}Z`,
  ].join(' ');
}

/**
 * The path for a preset, or `null` when this renderer has none.
 *
 * Grouped the way Office's gallery is, so a missing shape is easy to place.
 */
export function shapePath(preset, shape) {
  switch (preset) {
    /* ------------------------------------------------------------- lines */
    case 'line':
    case 'straightConnector1':
      return 'M0,0L100,100';
    case 'bentConnector3':
      return 'M0,0L50,0L50,100L100,100';
    case 'curvedConnector3':
      return 'M0,0C50,0 50,100 100,100';

    /* -------------------------------------------------------- rectangles */
    case 'rect':
    case 'flowChartProcess':
      return 'M0,0L100,0L100,100L0,100Z';
    case 'roundRect':
    case 'flowChartAlternateProcess': {
      const r = clamp(adj(shape, 'adj', 0.16667), 0, 0.5) * 100;
      return rounded([r, r, r, r]);
    }
    case 'round1Rect': {
      const r = clamp(adj(shape, 'adj', 0.16667), 0, 0.5) * 100;
      return rounded([0, r, 0, 0]);
    }
    case 'round2SameRect': {
      const r = clamp(adj(shape, 'adj1', 0.16667), 0, 0.5) * 100;
      return rounded([r, r, 0, 0]);
    }
    case 'round2DiagRect': {
      const r = clamp(adj(shape, 'adj1', 0.16667), 0, 0.5) * 100;
      return rounded([r, 0, r, 0]);
    }
    case 'snip1Rect': {
      const r = clamp(adj(shape, 'adj', 0.16667), 0, 0.5) * 100;
      return snipped([0, r, 0, 0]);
    }
    case 'snip2SameRect': {
      const r = clamp(adj(shape, 'adj1', 0.16667), 0, 0.5) * 100;
      return snipped([r, r, 0, 0]);
    }
    case 'snip2DiagRect': {
      const r = clamp(adj(shape, 'adj1', 0.16667), 0, 0.5) * 100;
      return snipped([r, 0, r, 0]);
    }
    case 'snipRoundRect': {
      const r = clamp(adj(shape, 'adj1', 0.16667), 0, 0.5) * 100;
      // One corner snipped, the adjacent one rounded.
      return [
        `M${r},0`,
        `L${100 - r},0L100,${r}`,
        'L100,100L0,100',
        `L0,${r}A${r},${r} 0 0 1 ${r},0`,
        'Z',
      ].join(' ');
    }
    case 'plaque': {
      const r = clamp(adj(shape, 'adj', 0.16667), 0, 0.5) * 100;
      return [
        `M0,${r}A${r},${r} 0 0 0 ${r},0`,
        `L${100 - r},0A${r},${r} 0 0 0 100,${r}`,
        `L100,${100 - r}A${r},${r} 0 0 0 ${100 - r},100`,
        `L${r},100A${r},${r} 0 0 0 0,${100 - r}Z`,
      ].join(' ');
    }
    case 'bevel':
    case 'frame': {
      const t = clamp(adj(shape, 'adj1', 0.125), 0.01, 0.49) * 100;
      return `M0,0L100,0L100,100L0,100Z M${t},${t}L${100 - t},${t}L${100 - t},${100 - t}L${t},${100 - t}Z`;
    }
    case 'halfFrame': {
      const t = clamp(adj(shape, 'adj1', 0.2), 0.01, 0.49) * 100;
      return `M0,0L100,0L${100 - t},${t}L${t},${t}L${t},${100 - t}L0,100Z`;
    }
    case 'corner': {
      const t = clamp(adj(shape, 'adj1', 0.5), 0.01, 0.99) * 100;
      return `M0,0L${t},0L${t},${100 - t}L100,${100 - t}L100,100L0,100Z`;
    }
    case 'diagStripe': {
      const t = clamp(adj(shape, 'adj', 0.5), 0.01, 0.99) * 100;
      return `M0,${t}L${t},0L100,0L100,${100 - t}L${100 - t},100L0,100Z`;
    }
    case 'foldedCorner': {
      const t = clamp(adj(shape, 'adj', 0.16667), 0.01, 0.5) * 100;
      return `M0,0L100,0L100,${100 - t}L${100 - t},100L0,100Z M${100 - t},100L${100 - t},${100 - t}L100,${100 - t}`;
    }

    /* ------------------------------------------------------ basic shapes */
    case 'ellipse':
    case 'flowChartConnector':
      return 'M50,0A50,50 0 1 1 50,100A50,50 0 1 1 50,0Z';
    case 'triangle': {
      const x = clamp(adj(shape, 'adj', 0.5), 0, 1) * 100;
      return `M${x},0L100,100L0,100Z`;
    }
    case 'rtTriangle':
      return 'M0,0L0,100L100,100Z';
    case 'parallelogram': {
      const t = clamp(adj(shape, 'adj', 0.25), 0, 1) * 100;
      return `M${t},0L100,0L${100 - t},100L0,100Z`;
    }
    case 'trapezoid':
    case 'flowChartManualOperation': {
      const t = clamp(adj(shape, 'adj', 0.25), 0, 0.5) * 100;
      return `M${t},0L${100 - t},0L100,100L0,100Z`;
    }
    case 'diamond':
    case 'flowChartDecision':
      return 'M50,0L100,50L50,100L0,50Z';
    case 'pentagon':
      return polygon(5);
    case 'hexagon':
      return polygon(6, 0);
    case 'heptagon':
      return polygon(7);
    case 'octagon':
      return polygon(8, 22.5);
    case 'decagon':
      return polygon(10);
    case 'dodecagon':
      return polygon(12, 15);
    case 'pie':
      // A quarter removed, which is Office's default sweep.
      return 'M50,50L100,50A50,50 0 1 1 50,0Z';
    case 'chord':
      return 'M85,15A50,50 0 1 1 15,85Z';
    case 'arc':
      return 'M100,50A50,50 0 0 0 50,0';
    case 'teardrop':
      return 'M50,0A50,50 0 1 1 50,100A50,50 0 0 1 0,50A50,50 0 0 1 50,0L100,0L100,50';
    case 'plus':
    case 'mathPlus': {
      const t = clamp(adj(shape, 'adj', 0.25), 0.01, 0.5) * 100;
      return `M${t},0L${100 - t},0L${100 - t},${t}L100,${t}L100,${100 - t}L${100 - t},${100 - t}L${100 - t},100L${t},100L${t},${100 - t}L0,${100 - t}L0,${t}L${t},${t}Z`;
    }
    case 'can':
      return 'M0,15A50,15 0 0 1 100,15L100,85A50,15 0 0 1 0,85Z M0,15A50,15 0 0 0 100,15';
    case 'cube':
      return 'M0,25L25,0L100,0L100,75L75,100L0,100Z M0,25L75,25L75,100 M75,25L100,0';
    case 'donut':
      return annulus(clamp(adj(shape, 'adj', 0.25), 0.05, 0.9));
    case 'noSmoking':
      return `${annulus(0.2)} M20,20L80,80`;
    case 'blockArc':
      return 'M0,50A50,50 0 0 1 100,50L75,50A25,25 0 0 0 25,50Z';
    case 'heart':
      return 'M50,100C10,65 0,45 0,28C0,10 15,0 30,0C40,0 47,6 50,14C53,6 60,0 70,0C85,0 100,10 100,28C100,45 90,65 50,100Z';
    case 'lightningBolt':
      return 'M35,0L75,0L55,38L80,38L25,100L40,55L15,55Z';
    case 'sun': {
      const path = ['M50,25A25,25 0 1 1 50,75A25,25 0 1 1 50,25Z'];
      for (let i = 0; i < 8; i++) {
        const a = (i * 45 * Math.PI) / 180;
        const [x1, y1] = [50 + 32 * Math.cos(a), 50 + 32 * Math.sin(a)];
        const [x2, y2] = [50 + 50 * Math.cos(a), 50 + 50 * Math.sin(a)];
        path.push(`M${x1},${y1}L${x2},${y2}`);
      }
      return path.join(' ');
    }
    case 'moon':
      return 'M75,0A50,50 0 1 0 75,100A62,62 0 0 1 75,0Z';
    case 'cloud':
      return 'M22,88C8,88 0,78 0,66C0,55 8,46 19,45C19,30 31,18 46,18C57,18 67,25 71,35C74,33 78,32 82,32C93,32 100,41 100,52C100,54 100,56 99,58C100,60 100,63 100,66C100,78 92,88 78,88Z';
    case 'smileyFace':
      return 'M50,0A50,50 0 1 1 50,100A50,50 0 1 1 50,0Z M30,35A5,7 0 1 1 30,36Z M70,35A5,7 0 1 1 70,36Z M25,62C35,80 65,80 75,62';
    case 'leftBrace':
      return 'M75,0C50,0 50,50 25,50C50,50 50,100 75,100';
    case 'rightBrace':
      return 'M25,0C50,0 50,50 75,50C50,50 50,100 25,100';
    case 'bracePair':
      return 'M25,0C8,0 8,50 0,50C8,50 8,100 25,100 M75,0C92,0 92,50 100,50C92,50 92,100 75,100';
    case 'bracketPair':
      return 'M25,0C10,0 0,10 0,25L0,75C0,90 10,100 25,100 M75,0C90,0 100,10 100,25L100,75C100,90 90,100 75,100';

    /* ---------------------------------------------------------- equation */
    case 'mathMinus': {
      const t = clamp(adj(shape, 'adj1', 0.23077), 0.01, 0.5) * 100;
      return `M0,${50 - t / 2}L100,${50 - t / 2}L100,${50 + t / 2}L0,${50 + t / 2}Z`;
    }
    case 'mathMultiply':
      return 'M15,0L50,35L85,0L100,15L65,50L100,85L85,100L50,65L15,100L0,85L35,50L0,15Z';
    case 'mathDivide':
      return 'M40,8A10,10 0 1 1 60,8A10,10 0 1 1 40,8Z M0,43L100,43L100,57L0,57Z M40,92A10,10 0 1 1 60,92A10,10 0 1 1 40,92Z';
    case 'mathEqual':
      return 'M0,28L100,28L100,42L0,42Z M0,58L100,58L100,72L0,72Z';
    case 'mathNotEqual':
      return 'M0,28L100,28L100,42L0,42Z M0,58L100,58L100,72L0,72Z M35,0L55,0L30,100L10,100Z';

    /* ------------------------------------------------------ block arrows */
    case 'rightArrow':
      return rightArrow(shape);
    case 'leftArrow':
    case 'upArrow':
    case 'downArrow':
      // Drawn once and rotated by the caller's transform.
      return rightArrow(shape);
    case 'leftRightArrow':
    case 'upDownArrow':
      return doubleArrow(shape);
    case 'quadArrow':
      return 'M50,0L70,25L57,25L57,43L75,43L75,30L100,50L75,70L75,57L57,57L57,75L70,75L50,100L30,75L43,75L43,57L25,57L25,70L0,50L25,30L25,43L43,43L43,25L30,25Z';
    case 'leftRightUpArrow':
      return 'M50,0L72,28L58,28L58,58L78,58L78,42L100,70L78,98L78,82L22,82L22,98L0,70L22,42L22,58L42,58L42,28L28,28Z';
    case 'bentArrow':
      return 'M0,100L0,55C0,35 15,25 35,25L70,25L70,0L100,32L70,64L70,40L38,40C30,40 25,45 25,55L25,100Z';
    case 'uturnArrow':
      return 'M0,100L0,40C0,18 16,0 38,0C60,0 76,18 76,40L76,62L100,62L62,100L24,62L48,62L48,40C48,34 44,28 38,28C32,28 28,34 28,40L28,100Z';
    case 'curvedRightArrow':
      return 'M0,10C55,10 90,35 90,50L100,50L75,80L50,50L60,50C60,40 35,28 0,28Z';
    case 'curvedLeftArrow':
      return 'M100,10C45,10 10,35 10,50L0,50L25,80L50,50L40,50C40,40 65,28 100,28Z';
    case 'stripedRightArrow':
      return `M0,32L5,32L5,68L0,68Z M9,32L16,32L16,68L9,68Z M20,32L${100 - 30},32L${100 - 30},10L100,50L${100 - 30},90L${100 - 30},68L20,68Z`;
    case 'notchedRightArrow':
      return 'M0,32L70,32L70,10L100,50L70,90L70,68L0,68L15,50Z';
    case 'homePlate': {
      const t = clamp(adj(shape, 'adj', 0.25), 0, 1) * 100;
      return `M0,0L${100 - t},0L100,50L${100 - t},100L0,100Z`;
    }
    case 'chevron': {
      const t = clamp(adj(shape, 'adj', 0.25), 0, 1) * 100;
      return `M0,0L${100 - t},0L100,50L${100 - t},100L0,100L${t},50Z`;
    }
    case 'circularArrow':
      return 'M50,8A42,42 0 1 1 12,62L28,55A26,26 0 1 0 50,24L50,38L22,18L50,0Z';
    case 'rightArrowCallout':
      return 'M0,0L70,0L70,20L80,20L80,5L100,50L80,95L80,80L70,80L70,100L0,100Z';
    case 'leftArrowCallout':
      return 'M100,0L30,0L30,20L20,20L20,5L0,50L20,95L20,80L30,80L30,100L100,100Z';
    case 'upArrowCallout':
      return 'M0,100L0,30L20,30L20,20L5,20L50,0L95,20L80,20L80,30L100,30L100,100Z';
    case 'downArrowCallout':
      return 'M0,0L0,70L20,70L20,80L5,80L50,100L95,80L80,80L80,70L100,70L100,0Z';

    /* --------------------------------------------------------- flowchart */
    case 'flowChartInputOutput':
      return 'M20,0L100,0L80,100L0,100Z';
    case 'flowChartPredefinedProcess':
      return 'M0,0L100,0L100,100L0,100Z M12,0L12,100 M88,0L88,100';
    case 'flowChartInternalStorage':
      return 'M0,0L100,0L100,100L0,100Z M0,12L100,12 M12,0L12,100';
    case 'flowChartDocument':
      return 'M0,0L100,0L100,86C75,102 25,70 0,86Z';
    case 'flowChartMultidocument':
      return 'M0,10L88,10L88,88C66,102 22,74 0,88Z M6,4L94,4L94,10 M12,0L100,0L100,82';
    case 'flowChartTerminator':
      return rounded([50, 50, 50, 50]);
    case 'flowChartPreparation':
      return 'M20,0L80,0L100,50L80,100L20,100L0,50Z';
    case 'flowChartManualInput':
      return 'M0,20L100,0L100,100L0,100Z';
    case 'flowChartOffpageConnector':
      return 'M0,0L100,0L100,80L50,100L0,80Z';
    case 'flowChartPunchedCard':
      return 'M20,0L100,0L100,100L0,100L0,20Z';
    case 'flowChartPunchedTape':
      return 'M0,10C25,-6 75,26 100,10L100,90C75,106 25,74 0,90Z';
    case 'flowChartSummingJunction':
      return 'M50,0A50,50 0 1 1 50,100A50,50 0 1 1 50,0Z M15,15L85,85 M85,15L15,85';
    case 'flowChartOr':
      return 'M50,0A50,50 0 1 1 50,100A50,50 0 1 1 50,0Z M50,0L50,100 M0,50L100,50';
    case 'flowChartCollate':
      return 'M0,0L100,0L0,100L100,100Z';
    case 'flowChartSort':
      return 'M50,0L100,50L50,100L0,50Z M0,50L100,50';
    case 'flowChartExtract':
      return 'M50,0L100,100L0,100Z';
    case 'flowChartMerge':
      return 'M0,0L100,0L50,100Z';
    case 'flowChartOnlineStorage':
      return 'M12,0L100,0C88,25 88,75 100,100L12,100A50,50 0 0 1 12,0Z';
    case 'flowChartDelay':
      return 'M0,0L50,0A50,50 0 0 1 50,100L0,100Z';
    case 'flowChartMagneticTape':
      return 'M50,0A50,50 0 1 1 78,92L100,92L100,100L50,100A50,50 0 0 1 50,0Z';
    case 'flowChartMagneticDisk':
      return 'M0,15A50,15 0 0 1 100,15L100,85A50,15 0 0 1 0,85Z M0,15A50,15 0 0 0 100,15';
    case 'flowChartMagneticDrum':
      return 'M15,0A15,50 0 0 0 15,100L85,100A15,50 0 0 0 85,0Z M85,0A15,50 0 0 1 85,100';
    case 'flowChartDisplay':
      return 'M15,0L85,0A25,50 0 0 1 85,100L15,100L0,50Z';

    /* -------------------------------------------------- stars and banners */
    case 'star4':
      return star(4, 0.15);
    case 'star5':
      return star(5, 0.38);
    case 'star6':
      return star(6, 0.58);
    case 'star7':
      return star(7, 0.62);
    case 'star8':
      return star(8, 0.66);
    case 'star10':
      return star(10, 0.72);
    case 'star12':
      return star(12, 0.76);
    case 'star16':
      return star(16, 0.82);
    case 'star24':
      return star(24, 0.88);
    case 'star32':
      return star(32, 0.92);
    case 'irregularSeal1':
      return 'M11,32L0,17L18,10L12,0L30,8L36,0L48,10L62,0L64,12L82,3L80,18L100,21L88,35L100,46L82,54L94,68L74,66L76,84L58,74L54,90L42,78L30,90L26,74L10,80L14,64L0,58L10,46Z';
    case 'irregularSeal2':
      return 'M20,28L4,20L16,10L6,0L24,4L30,0L40,8L52,0L58,10L74,2L74,16L92,14L86,28L100,34L88,46L100,58L84,62L92,76L74,72L76,88L58,80L52,92L40,82L28,92L24,78L8,84L14,68L0,60L12,48Z';
    case 'ribbon':
      return 'M0,0L20,0L20,60L50,60L80,60L80,0L100,0L100,80L70,80L80,100L50,88L20,100L30,80L0,80Z';
    case 'ribbon2':
      return 'M0,20L20,0L30,20L50,12L70,20L80,0L100,20L100,100L80,100L80,40L20,40L20,100L0,100Z';
    case 'ellipseRibbon':
    case 'ellipseRibbon2':
      return 'M0,20C30,0 70,0 100,20L100,80C70,60 30,60 0,80Z';
    case 'verticalScroll':
      return 'M12,0L100,0L100,88C100,95 94,100 88,100L0,100L0,12C0,5 6,0 12,0Z M12,0C6,0 0,5 0,12L12,12Z';
    case 'horizontalScroll':
      return 'M0,12C0,5 5,0 12,0L100,0L100,88C100,95 95,100 88,100L0,100Z M100,0C100,0 88,0 88,12L100,12Z';
    case 'wave':
      return 'M0,20C25,0 75,40 100,20L100,80C75,100 25,60 0,80Z';
    case 'doubleWave':
      return 'M0,20C12,4 38,36 50,20C62,4 88,36 100,20L100,80C88,96 62,64 50,80C38,96 12,64 0,80Z';

    /* ---------------------------------------------------------- callouts */
    case 'wedgeRectCallout':
      return callout(shape, 0);
    case 'wedgeRoundRectCallout':
      return callout(shape, 16);
    case 'wedgeEllipseCallout': {
      const x = 50 + clamp(adj(shape, 'adj1', -0.2), -1.5, 1.5) * 100;
      const y = 50 + clamp(adj(shape, 'adj2', 0.8), -1.5, 1.5) * 100;
      return `M50,0A50,50 0 1 1 50,100A50,50 0 1 1 50,0Z M38,95L${x},${y}L58,92Z`;
    }
    case 'cloudCallout':
      return 'M22,72C8,72 0,63 0,53C0,44 8,36 19,36C19,23 31,12 46,12C57,12 67,19 71,27C74,25 78,24 82,24C93,24 100,32 100,42C100,44 100,46 99,47C100,49 100,51 100,53C100,63 92,72 78,72Z M24,78A6,5 0 1 1 24,79Z M14,90A5,4 0 1 1 14,91Z';
    case 'borderCallout1':
    case 'callout1':
      return 'M0,0L100,0L100,60L0,60Z M20,60L10,100';
    case 'borderCallout2':
      return 'M0,0L100,0L100,50L0,50Z M20,50L15,75L5,100';
    case 'borderCallout3':
      return 'M0,0L100,0L100,50L0,50Z M20,50L20,75L5,75L5,100';
    case 'accentCallout1':
      return 'M0,0L100,0L100,60L0,60Z M4,0L4,60';

    default:
      return null;
  }
}

/** Presets that are open lines rather than closed regions. */
const OPEN = new Set([
  'line',
  'straightConnector1',
  'bentConnector3',
  'curvedConnector3',
  'arc',
  'leftBrace',
  'rightBrace',
  'bracePair',
  'bracketPair',
]);

/** Presets whose drawn path is the right-pointing one, rotated. */
const ROTATED = { leftArrow: 180, upArrow: -90, downArrow: 90, upDownArrow: -90 };

export function isOpenShape(preset) {
  return OPEN.has(preset);
}

export function presetRotation(preset) {
  return ROTATED[preset] ?? 0;
}

/** True when this renderer knows the preset's outline. */
export function canDraw(preset) {
  return shapePath(preset, null) !== null;
}
