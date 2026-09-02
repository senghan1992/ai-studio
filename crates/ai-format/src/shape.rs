//! Shapes.
//!
//! The vocabulary is OOXML's own `prstGeom` preset names — `roundRect`,
//! `rightArrow`, `flowChartDecision` and so on. Adopting Office's names rather
//! than inventing a parallel set is what makes a `.pptx` open here looking like
//! itself and go back out unchanged.
//!
//! A preset we cannot draw is still **stored under its real name**. Losing the
//! name would silently downgrade a shape to a rectangle forever; keeping it means
//! the file round-trips and the renderer can learn the shape later.

use indexmap::IndexMap;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

/// Which drawer of Office's shape gallery a preset lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShapeGroup {
    Lines,
    Rectangles,
    Basic,
    BlockArrows,
    Equation,
    Flowchart,
    StarsBanners,
    Callouts,
}

impl ShapeGroup {
    pub fn label(self) -> &'static str {
        match self {
            ShapeGroup::Lines => "선",
            ShapeGroup::Rectangles => "사각형",
            ShapeGroup::Basic => "기본 도형",
            ShapeGroup::BlockArrows => "블록 화살표",
            ShapeGroup::Equation => "수식 도형",
            ShapeGroup::Flowchart => "순서도",
            ShapeGroup::StarsBanners => "별 및 배너",
            ShapeGroup::Callouts => "설명선",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ShapeGroup::Lines => "lines",
            ShapeGroup::Rectangles => "rectangles",
            ShapeGroup::Basic => "basic",
            ShapeGroup::BlockArrows => "blockArrows",
            ShapeGroup::Equation => "equation",
            ShapeGroup::Flowchart => "flowchart",
            ShapeGroup::StarsBanners => "starsBanners",
            ShapeGroup::Callouts => "callouts",
        }
    }
}

pub struct Preset {
    /// The `prstGeom` name, exactly as OOXML spells it.
    pub name: &'static str,
    /// The label Office uses in Korean, so the gallery and `AI.md` read the same.
    pub label: &'static str,
    pub group: ShapeGroup,
}

macro_rules! presets {
    ($($group:ident: $($name:literal => $label:literal),+ ;)+) => {
        &[ $($( Preset { name: $name, label: $label, group: ShapeGroup::$group } ),+),+ ]
    };
}

/// The shape gallery.
///
/// Names are ECMA-376 preset geometries. The list covers what Office's gallery
/// offers in the drawers people actually open; anything outside it still stores
/// and exports correctly, it simply draws as its bounding rectangle here.
pub static PRESETS: &[Preset] = presets! {
    Lines:
        "line" => "선",
        "straightConnector1" => "직선 연결선",
        "bentConnector3" => "꺾인 연결선",
        "curvedConnector3" => "곡선 연결선";
    Rectangles:
        "rect" => "직사각형",
        "roundRect" => "둥근 직사각형",
        "round1Rect" => "한쪽 모서리가 둥근 사각형",
        "round2SameRect" => "위쪽 모서리가 둥근 사각형",
        "round2DiagRect" => "대각선 방향 모서리가 둥근 사각형",
        "snip1Rect" => "한쪽 모서리가 잘린 사각형",
        "snip2SameRect" => "위쪽 모서리가 잘린 사각형",
        "snip2DiagRect" => "대각선 방향 모서리가 잘린 사각형",
        "snipRoundRect" => "한쪽 모서리는 잘리고 다른 쪽은 둥근 사각형";
    Basic:
        "ellipse" => "타원",
        "triangle" => "이등변 삼각형",
        "rtTriangle" => "직각 삼각형",
        "parallelogram" => "평행사변형",
        "trapezoid" => "사다리꼴",
        "diamond" => "다이아몬드",
        "pentagon" => "정오각형",
        "hexagon" => "육각형",
        "heptagon" => "칠각형",
        "octagon" => "팔각형",
        "decagon" => "십각형",
        "dodecagon" => "십이각형",
        "pie" => "파이",
        "chord" => "현",
        "teardrop" => "눈물 방울",
        "frame" => "액자",
        "halfFrame" => "반액자",
        "corner" => "L 도형",
        "diagStripe" => "대각선 방향 줄무늬",
        "plus" => "십자형",
        "plaque" => "명판",
        "can" => "원통",
        "cube" => "정육면체",
        "bevel" => "액자 모양",
        "donut" => "도넛",
        "noSmoking" => "금지",
        "blockArc" => "막힌 원호",
        "foldedCorner" => "한쪽 모서리가 접힌 사각형",
        "smileyFace" => "웃는 얼굴",
        "heart" => "하트",
        "lightningBolt" => "번개",
        "sun" => "해",
        "moon" => "달",
        "cloud" => "구름",
        "arc" => "원호",
        "bracePair" => "중괄호",
        "leftBrace" => "왼쪽 중괄호",
        "rightBrace" => "오른쪽 중괄호",
        "bracketPair" => "대괄호";
    BlockArrows:
        "rightArrow" => "오른쪽 화살표",
        "leftArrow" => "왼쪽 화살표",
        "upArrow" => "위쪽 화살표",
        "downArrow" => "아래쪽 화살표",
        "leftRightArrow" => "왼쪽/오른쪽 화살표",
        "upDownArrow" => "위쪽/아래쪽 화살표",
        "quadArrow" => "사방 화살표",
        "leftRightUpArrow" => "왼쪽/오른쪽/위쪽 화살표",
        "bentArrow" => "굽은 화살표",
        "uturnArrow" => "U턴 화살표",
        "curvedRightArrow" => "오른쪽으로 구부러진 화살표",
        "curvedLeftArrow" => "왼쪽으로 구부러진 화살표",
        "stripedRightArrow" => "줄무늬가 있는 오른쪽 화살표",
        "notchedRightArrow" => "빗면이 있는 오른쪽 화살표",
        "homePlate" => "오각형",
        "chevron" => "갈매기형 수장",
        "rightArrowCallout" => "오른쪽 화살표 설명선",
        "leftArrowCallout" => "왼쪽 화살표 설명선",
        "upArrowCallout" => "위쪽 화살표 설명선",
        "downArrowCallout" => "아래쪽 화살표 설명선",
        "circularArrow" => "원형 화살표";
    Equation:
        "mathPlus" => "더하기",
        "mathMinus" => "빼기",
        "mathMultiply" => "곱하기",
        "mathDivide" => "나누기",
        "mathEqual" => "등호",
        "mathNotEqual" => "같지 않음";
    Flowchart:
        "flowChartProcess" => "처리",
        "flowChartAlternateProcess" => "대체 처리",
        "flowChartDecision" => "판단",
        "flowChartInputOutput" => "데이터",
        "flowChartPredefinedProcess" => "미리 정의된 처리",
        "flowChartInternalStorage" => "내부 저장소",
        "flowChartDocument" => "문서",
        "flowChartMultidocument" => "여러 문서",
        "flowChartTerminator" => "종료",
        "flowChartPreparation" => "준비",
        "flowChartManualInput" => "수동 입력",
        "flowChartManualOperation" => "수동 연산",
        "flowChartConnector" => "연결선",
        "flowChartOffpageConnector" => "페이지 외 연결선",
        "flowChartPunchedCard" => "카드",
        "flowChartPunchedTape" => "천공 테이프",
        "flowChartSummingJunction" => "논리합",
        "flowChartOr" => "OR",
        "flowChartCollate" => "병합",
        "flowChartSort" => "정렬",
        "flowChartExtract" => "추출",
        "flowChartMerge" => "병합 저장",
        "flowChartOnlineStorage" => "저장 데이터",
        "flowChartDelay" => "지연",
        "flowChartMagneticTape" => "순차 접근 저장소",
        "flowChartMagneticDisk" => "자기 디스크",
        "flowChartMagneticDrum" => "직접 접근 저장소",
        "flowChartDisplay" => "표시";
    StarsBanners:
        "irregularSeal1" => "폭발 1",
        "irregularSeal2" => "폭발 2",
        "star4" => "4각 별",
        "star5" => "5각 별",
        "star6" => "6각 별",
        "star7" => "7각 별",
        "star8" => "8각 별",
        "star10" => "10각 별",
        "star12" => "12각 별",
        "star16" => "16각 별",
        "star24" => "24각 별",
        "star32" => "32각 별",
        "ribbon" => "아래로 구부러진 리본",
        "ribbon2" => "위로 구부러진 리본",
        "ellipseRibbon" => "물결 리본",
        "ellipseRibbon2" => "위로 물결치는 리본",
        "verticalScroll" => "세로 스크롤",
        "horizontalScroll" => "가로 스크롤",
        "wave" => "물결",
        "doubleWave" => "이중 물결";
    Callouts:
        "wedgeRectCallout" => "사각형 설명선",
        "wedgeRoundRectCallout" => "둥근 사각형 설명선",
        "wedgeEllipseCallout" => "타원형 설명선",
        "cloudCallout" => "구름 설명선",
        "borderCallout1" => "선 설명선 1",
        "borderCallout2" => "선 설명선 2",
        "borderCallout3" => "선 설명선 3",
        "accentCallout1" => "강조선 설명선 1",
        "callout1" => "선 테두리 설명선 1";
};

static BY_NAME: Lazy<IndexMap<&'static str, &'static Preset>> =
    Lazy::new(|| PRESETS.iter().map(|p| (p.name, p)).collect());

pub fn preset(name: &str) -> Option<&'static Preset> {
    BY_NAME.get(name).copied()
}

/// The Korean name for a preset, or the raw `prstGeom` name when it is one we do
/// not have a label for — better an unfamiliar name in `AI.md` than a wrong one.
pub fn preset_label(name: &str) -> &str {
    preset(name).map(|p| p.label).unwrap_or(name)
}

/// A solid fill. Gradients and picture fills are read as their first stop so an
/// imported shape keeps roughly the right colour rather than turning transparent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub color: String,
    /// 0..=100. Office stores transparency; this is the complement.
    #[serde(default = "full", skip_serializing_if = "is_full")]
    pub opacity: f64,
}

fn full() -> f64 {
    100.0
}
fn is_full(v: &f64) -> bool {
    *v >= 100.0
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Dash {
    #[default]
    Solid,
    Dot,
    Dash,
    DashDot,
    LongDash,
}

impl Dash {
    /// The OOXML `a:prstDash` value.
    pub fn as_ooxml(self) -> &'static str {
        match self {
            Dash::Solid => "solid",
            Dash::Dot => "sysDot",
            Dash::Dash => "dash",
            Dash::DashDot => "dashDot",
            Dash::LongDash => "lgDash",
        }
    }

    pub fn from_ooxml(value: &str) -> Dash {
        match value {
            "dot" | "sysDot" | "sysDash" => Dash::Dot,
            "dash" | "sysDashDot" => Dash::Dash,
            "dashDot" | "lgDashDot" | "lgDashDotDot" | "sysDashDotDot" => Dash::DashDot,
            "lgDash" => Dash::LongDash,
            _ => Dash::Solid,
        }
    }

    /// An SVG `stroke-dasharray` for a given stroke width.
    pub fn svg_dasharray(self, width: f64) -> Option<String> {
        let w = width.max(0.5);
        Some(match self {
            Dash::Solid => return None,
            Dash::Dot => format!("{} {}", w, w * 2.0),
            Dash::Dash => format!("{} {}", w * 4.0, w * 3.0),
            Dash::DashDot => format!("{} {} {} {}", w * 4.0, w * 3.0, w, w * 3.0),
            Dash::LongDash => format!("{} {}", w * 8.0, w * 3.0),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub color: String,
    /// Stroke width in px.
    #[serde(default = "one_px")]
    pub width: f64,
    #[serde(default, skip_serializing_if = "is_solid")]
    pub dash: Dash,
}

fn one_px() -> f64 {
    1.0
}
fn is_solid(d: &Dash) -> bool {
    *d == Dash::Solid
}

/// Everything about a shape that is not its position or its text.
///
/// `fill: None` means no fill and `line: None` means no outline — the distinction
/// matters, because a shape with neither is invisible in Office too and we must
/// not "helpfully" give it one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeSpec {
    /// A `prstGeom` name. Preserved verbatim even when unknown to the renderer.
    pub preset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Line>,
    /// Clockwise degrees.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: f64,
    #[serde(rename = "flipH", default, skip_serializing_if = "is_false")]
    pub flip_h: bool,
    #[serde(rename = "flipV", default, skip_serializing_if = "is_false")]
    pub flip_v: bool,
    /// `prstGeom` adjust handles, in OOXML's 1/100000 units (`adj`, `adj1`, …).
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub adjust: IndexMap<String, f64>,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}
fn is_false(v: &bool) -> bool {
    !*v
}

impl Default for ShapeSpec {
    fn default() -> Self {
        ShapeSpec {
            preset: "rect".to_string(),
            fill: Some(Fill {
                color: "#dbeafe".to_string(),
                opacity: 100.0,
            }),
            line: None,
            rotation: 0.0,
            flip_h: false,
            flip_v: false,
            adjust: IndexMap::new(),
        }
    }
}

impl ShapeSpec {
    pub fn label(&self) -> &str {
        preset_label(&self.preset)
    }

    /// True when the renderer here has a path for this preset.
    pub fn is_known(&self) -> bool {
        preset(&self.preset).is_some()
    }
}

/// The gallery, as the editor's shape picker needs it.
pub fn gallery() -> Vec<(ShapeGroup, Vec<&'static Preset>)> {
    let mut out: Vec<(ShapeGroup, Vec<&'static Preset>)> = Vec::new();
    for p in PRESETS {
        match out.last_mut() {
            Some((group, list)) if *group == p.group => list.push(p),
            _ => out.push((p.group, vec![p])),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for p in PRESETS {
            assert!(seen.insert(p.name), "duplicate preset {}", p.name);
        }
    }

    #[test]
    fn the_gallery_covers_every_office_drawer() {
        let groups: Vec<ShapeGroup> = gallery().into_iter().map(|(g, _)| g).collect();
        for expected in [
            ShapeGroup::Lines,
            ShapeGroup::Rectangles,
            ShapeGroup::Basic,
            ShapeGroup::BlockArrows,
            ShapeGroup::Equation,
            ShapeGroup::Flowchart,
            ShapeGroup::StarsBanners,
            ShapeGroup::Callouts,
        ] {
            assert!(groups.contains(&expected), "{expected:?} missing");
        }
        assert!(PRESETS.len() > 100, "only {} presets", PRESETS.len());
    }

    #[test]
    fn an_unknown_preset_keeps_its_name() {
        let spec = ShapeSpec {
            preset: "someFuturePreset".into(),
            ..ShapeSpec::default()
        };
        assert!(!spec.is_known());
        // The label falls back to the raw name rather than lying about the shape.
        assert_eq!(spec.label(), "someFuturePreset");
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains("someFuturePreset"), "{json}");
    }

    #[test]
    fn labels_match_office_korean() {
        assert_eq!(preset_label("roundRect"), "둥근 직사각형");
        assert_eq!(preset_label("flowChartDecision"), "판단");
        assert_eq!(preset_label("star5"), "5각 별");
        assert_eq!(preset_label("wedgeEllipseCallout"), "타원형 설명선");
    }

    #[test]
    fn no_fill_and_no_line_survive_the_round_trip() {
        let spec = ShapeSpec {
            preset: "line".into(),
            fill: None,
            line: Some(Line {
                color: "#000000".into(),
                width: 2.0,
                dash: Dash::Dash,
            }),
            ..ShapeSpec::default()
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(
            !json.contains("fill"),
            "an absent fill must not be invented: {json}"
        );
        let back: ShapeSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn dash_styles_map_both_ways() {
        for dash in [
            Dash::Solid,
            Dash::Dot,
            Dash::Dash,
            Dash::DashDot,
            Dash::LongDash,
        ] {
            assert_eq!(Dash::from_ooxml(dash.as_ooxml()), dash, "{dash:?}");
        }
        // Office has more dash values than we model; each lands somewhere sane.
        assert_eq!(Dash::from_ooxml("lgDashDotDot"), Dash::DashDot);
        assert_eq!(Dash::from_ooxml("nonsense"), Dash::Solid);
        assert_eq!(Dash::Solid.svg_dasharray(1.0), None);
        assert_eq!(Dash::Dot.svg_dasharray(2.0).as_deref(), Some("2 4"));
    }
}
