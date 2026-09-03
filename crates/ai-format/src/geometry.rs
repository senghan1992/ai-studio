//! Pixel geometry, and translating it into words a model can reason about.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    pub w: f64,
    pub h: f64,
    #[serde(default = "white")]
    pub bg: BgColor,
}

/// The background is a colour string; a separate type keeps `Canvas: Copy`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub struct BgColor([u8; 12]);

fn white() -> BgColor {
    BgColor::from("#ffffff".to_string())
}

impl From<String> for BgColor {
    fn from(s: String) -> Self {
        let mut buf = [0u8; 12];
        for (slot, b) in buf.iter_mut().zip(s.bytes()) {
            *slot = b;
        }
        BgColor(buf)
    }
}

impl From<BgColor> for String {
    fn from(c: BgColor) -> String {
        c.as_str().to_string()
    }
}

impl BgColor {
    pub fn as_str(&self) -> &str {
        let end = self.0.iter().position(|b| *b == 0).unwrap_or(self.0.len());
        std::str::from_utf8(&self.0[..end]).unwrap_or("#ffffff")
    }
}

impl Default for Canvas {
    fn default() -> Self {
        DEFAULT_CANVAS
    }
}

pub const DEFAULT_CANVAS: Canvas = Canvas {
    w: 1280.0,
    h: 720.0,
    bg: BgColor(*b"#ffffff\0\0\0\0\0"),
};

/// A positioned box on a slide.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Box {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub z: f64,
}

const V_BANDS: [(f64, &str); 3] = [(0.28, "상단"), (0.68, "중앙"), (f64::INFINITY, "하단")];
const H_BANDS: [(f64, &str); 3] = [(0.3, "좌측"), (0.7, "중앙"), (f64::INFINITY, "우측")];

fn band(ratio: f64, bands: &[(f64, &'static str); 3]) -> &'static str {
    for (limit, label) in bands {
        if ratio < *limit {
            return label;
        }
    }
    bands[bands.len() - 1].1
}

/// Translate pixel geometry into a phrase an LLM can reason about.
///
/// This is the single most useful thing the digest does: models handle
/// "상단 중앙, 슬라이드 폭의 85%" far better than "x=96 y=220 w=1088".
pub fn position_phrase(b: &Box, canvas: &Canvas) -> String {
    let cx = (b.x + b.w / 2.0) / canvas.w;
    let cy = (b.y + b.h / 2.0) / canvas.h;
    let v = band(cy, &V_BANDS);
    let h = band(cx, &H_BANDS);
    let where_ = if v == "중앙" && h == "중앙" {
        "정중앙".to_string()
    } else {
        format!("{v} {h}")
    };

    let width_pct = js_round(b.w / canvas.w * 100.0);
    let size = if width_pct >= 85.0 {
        "전체 폭".to_string()
    } else if width_pct <= 35.0 {
        "좁은 폭".to_string()
    } else {
        format!("폭 {}%", width_pct as i64)
    };
    format!("{where_}, {size}")
}

/// `Math.round` — half away from zero, unlike Rust's `f64::round` on ties
/// for negatives (`(-0.5).round()` is -1 in Rust and -0 in JS).
pub fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// Reading order: top-to-bottom, then left-to-right, with a row tolerance.
///
/// Rows are formed first — a box joins the current row while it starts within
/// the tolerance of the row's first box — and only then is everything sorted
/// by (row, x, y). Comparing pairs with a tolerance directly is not a total
/// order (A≈B and B≈C do not make A≈C), and a slide of forty staggered
/// diagram boxes made the sort detect that and abort the whole process.
pub fn reading_order<T: Clone>(
    items: &[T],
    box_of: impl Fn(&T) -> Box,
    row_tolerance: f64,
) -> Vec<T> {
    let boxes: Vec<Box> = items.iter().map(&box_of).collect();
    let mut by_y: Vec<usize> = (0..items.len()).collect();
    by_y.sort_by(|&a, &b| boxes[a].y.total_cmp(&boxes[b].y).then_with(|| a.cmp(&b)));
    let mut row = vec![0usize; items.len()];
    let mut current = 0usize;
    let mut row_top = f64::NEG_INFINITY;
    for &i in &by_y {
        let y = boxes[i].y;
        let gap = y - row_top;
        if gap.is_nan() || gap > row_tolerance {
            current += 1;
            row_top = y;
        }
        row[i] = current;
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    // The original index breaks ties, so declaration order is kept for boxes
    // that coincide — matching Array.prototype.sort in every current engine.
    order.sort_by(|&a, &b| {
        row[a]
            .cmp(&row[b])
            .then_with(|| boxes[a].x.total_cmp(&boxes[b].x))
            .then_with(|| boxes[a].y.total_cmp(&boxes[b].y))
            .then_with(|| a.cmp(&b))
    });
    order.into_iter().map(|i| items[i].clone()).collect()
}

/// Vertical stack fallback for blocks with no saved geometry, so a markdown-only
/// slide still renders sensibly instead of piling everything at the origin.
pub fn auto_layout(index: usize, total: usize, canvas: &Canvas) -> Box {
    let margin = js_round(canvas.w * 0.075);
    let top = js_round(canvas.h * 0.14);
    let gap = 24.0;
    let usable = canvas.h - top - margin;
    let spread = gap * (total.saturating_sub(1)) as f64;
    let h = f64::max(64.0, js_round((usable - spread) / total.max(1) as f64));
    Box {
        x: margin,
        y: top + index as f64 * (h + gap),
        w: canvas.w - margin * 2.0,
        h,
        z: index as f64 + 1.0,
    }
}

/// Clamp a box inside the canvas but never below a usable minimum size.
pub fn clamp_box(b: Box, canvas: &Canvas) -> Box {
    let w = js_round(b.w).max(40.0).min(canvas.w);
    let h = js_round(b.h).max(28.0).min(canvas.h);
    Box {
        x: js_round(b.x).max(0.0).min(canvas.w - w),
        y: js_round(b.y).max(0.0).min(canvas.h - h),
        w,
        h,
        z: b.z,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: f64, y: f64, w: f64, h: f64) -> Box {
        Box { x, y, w, h, z: 1.0 }
    }

    #[test]
    fn positions_become_phrases() {
        let c = DEFAULT_CANVAS;
        // The README's own example: x=96 y=64 w=1088 h=70 -> 상단 중앙, 전체 폭
        assert_eq!(
            position_phrase(&b(96.0, 64.0, 1088.0, 70.0), &c),
            "상단 중앙, 전체 폭"
        );
        assert_eq!(
            position_phrase(&b(96.0, 300.0, 614.0, 100.0), &c),
            "정중앙, 폭 48%"
        );
        assert_eq!(
            position_phrase(&b(0.0, 600.0, 300.0, 80.0), &c),
            "하단 좌측, 좁은 폭"
        );
        assert_eq!(
            position_phrase(&b(980.0, 0.0, 300.0, 80.0), &c),
            "상단 우측, 좁은 폭"
        );
    }

    #[test]
    fn auto_layout_stacks_without_overlap() {
        let c = DEFAULT_CANVAS;
        let first = auto_layout(0, 3, &c);
        let second = auto_layout(1, 3, &c);
        assert_eq!(first.x, 96.0);
        assert_eq!(first.w, 1088.0);
        assert!(second.y >= first.y + first.h, "{first:?} {second:?}");
        // The last box must still fit on the canvas.
        let last = auto_layout(2, 3, &c);
        assert!(last.y + last.h <= c.h, "{last:?}");
    }

    #[test]
    fn clamping_keeps_boxes_on_canvas_and_usable() {
        let c = DEFAULT_CANVAS;
        assert_eq!(clamp_box(b(-50.0, -50.0, 100.0, 100.0), &c).x, 0.0);
        assert_eq!(clamp_box(b(2000.0, 0.0, 100.0, 100.0), &c).x, 1180.0);
        // A box smaller than the minimum grows rather than vanishing.
        let tiny = clamp_box(b(0.0, 0.0, 1.0, 1.0), &c);
        assert_eq!((tiny.w, tiny.h), (40.0, 28.0));
    }

    #[test]
    fn reading_order_is_rows_then_columns() {
        let boxes = vec![
            b(600.0, 100.0, 100.0, 50.0),
            b(96.0, 110.0, 100.0, 50.0),
            b(96.0, 400.0, 100.0, 50.0),
        ];
        let ordered = reading_order(&boxes, |x| *x, 40.0);
        // The first two are the same row within tolerance, so x decides.
        assert_eq!(ordered[0].x, 96.0);
        assert_eq!(ordered[1].x, 600.0);
        assert_eq!(ordered[2].y, 400.0);
    }

    #[test]
    fn canvas_serializes_with_its_background() {
        let json = serde_json::to_string(&DEFAULT_CANVAS).unwrap();
        assert_eq!(json, r##"{"w":1280.0,"h":720.0,"bg":"#ffffff"}"##);
        let back: Canvas = serde_json::from_str(r##"{"w":1280,"h":720,"bg":"#101014"}"##).unwrap();
        assert_eq!(back.bg.as_str(), "#101014");
    }

    #[test]
    fn staggered_boxes_sort_without_tripping_the_total_order_check() {
        // Forty boxes each 30px lower than the last: with a 40px tolerance
        // every neighbour "ties" while the ends do not, which is exactly the
        // non-transitive comparison Rust's sort rejects. Rows must be formed
        // first, and the order must be deterministic.
        let boxes: Vec<Box> = (0..40)
            .map(|i| Box {
                x: ((i * 37) % 11) as f64 * 50.0,
                y: i as f64 * 30.0,
                w: 40.0,
                h: 20.0,
                z: 0.0,
            })
            .collect();
        let ordered = reading_order(&boxes, |b| *b, 40.0);
        assert_eq!(ordered.len(), 40);
        // Rows are 40px tall anchored at the row's first box: ys 0 and 30 share
        // a row and 60 starts the next, so the first two are those two boxes in
        // x order, and no box ever comes more than a row after a later one.
        let mut first_two = [ordered[0].y, ordered[1].y];
        first_two.sort_by(f64::total_cmp);
        assert_eq!(first_two, [0.0, 30.0]);
        assert!(ordered[0].x <= ordered[1].x);
        assert!(ordered.windows(2).all(|w| w[0].y <= w[1].y + 40.0));
    }
}
