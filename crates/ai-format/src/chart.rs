//! Chart specs: the data half of a chart.
//!
//! A chart in this format is never a picture on disk — it is a spec that either
//! carries its numbers inline or points at a sheet range. That is what lets
//! `AI.md` state the figures a model needs, and what lets the PowerPoint export
//! emit a native, editable chart. Drawing is the view layer's job.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

use ai_formula::evaluate::{display_value, Cell};
use ai_formula::refs::{parse_range, to_ref};

pub const CHART_TYPES: [&str; 6] = ["column", "bar", "line", "area", "pie", "donut"];

/// A ninth series is never a generated hue — it folds into "기타".
pub const MAX_SERIES: usize = 8;

/// Categorical palette, fixed order, never cycled.
///
/// Both rows are selected — the dark row is the same eight hues re-stepped for a
/// dark surface, not an automatic inversion. Validated against both surfaces:
/// worst adjacent CVD ΔE 9.1 light / 8.4 dark, worst adjacent normal-vision
/// ΔE 19.6 / 19.3.
pub const PALETTE_LIGHT: [&str; 8] = [
    "#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#4a3aa7", "#e34948",
];
pub const PALETTE_DARK: [&str; 8] = [
    "#3987e5", "#d95926", "#199e70", "#c98500", "#d55181", "#008300", "#9085e9", "#e66767",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartType {
    Column,
    Bar,
    Line,
    Area,
    Pie,
    Donut,
}

impl ChartType {
    pub fn as_str(self) -> &'static str {
        match self {
            ChartType::Column => "column",
            ChartType::Bar => "bar",
            ChartType::Line => "line",
            ChartType::Area => "area",
            ChartType::Pie => "pie",
            ChartType::Donut => "donut",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ChartType::Column => "세로 막대",
            ChartType::Bar => "가로 막대",
            ChartType::Line => "꺾은선",
            ChartType::Area => "영역",
            ChartType::Pie => "원형",
            ChartType::Donut => "도넛",
        }
    }

    fn from_str(s: &str) -> Option<ChartType> {
        Some(match s {
            "column" => ChartType::Column,
            "bar" => ChartType::Bar,
            "line" => ChartType::Line,
            "area" => ChartType::Area,
            "pie" => ChartType::Pie,
            "donut" => ChartType::Donut,
            _ => return None,
        })
    }

    pub fn is_radial(self) -> bool {
        matches!(self, ChartType::Pie | ChartType::Donut)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    Columns,
    Rows,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueLabels {
    None,
    Max,
    Ends,
    All,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Series {
    #[serde(default)]
    pub name: String,
    /// `null` is a gap, not a zero.
    pub values: Vec<Option<f64>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartOptions {
    #[serde(default)]
    pub stacked: bool,
    #[serde(default = "yes")]
    pub legend: bool,
    #[serde(default = "yes")]
    pub grid: bool,
    /// Selective by default: a number on every point is noise.
    #[serde(default = "default_value_labels", rename = "valueLabels")]
    pub value_labels: ValueLabels,
    #[serde(default, rename = "yTitle")]
    pub y_title: String,
    #[serde(default, rename = "numberFormat")]
    pub number_format: String,
}

fn yes() -> bool {
    true
}
fn default_value_labels() -> ValueLabels {
    ValueLabels::Max
}

impl Default for ChartOptions {
    fn default() -> Self {
        Self {
            stacked: false,
            legend: true,
            grid: true,
            value_labels: ValueLabels::Max,
            y_title: String::new(),
            number_format: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartSpec {
    #[serde(rename = "type")]
    pub chart_type: ChartType,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub series: Vec<Series>,
    /// The sheet range the chart is a view of, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    #[serde(default = "default_orientation")]
    pub orientation: Orientation,
    #[serde(default)]
    pub options: ChartOptions,
}

fn default_orientation() -> Orientation {
    Orientation::Columns
}

impl Default for ChartSpec {
    fn default() -> Self {
        Self {
            chart_type: ChartType::Column,
            title: String::new(),
            labels: Vec::new(),
            series: Vec::new(),
            range: None,
            orientation: Orientation::Columns,
            options: ChartOptions::default(),
        }
    }
}

/// Fill in defaults and clamp anything that would break a renderer.
pub fn normalize_spec(raw: &serde_json::Value) -> ChartSpec {
    let obj = raw.as_object();
    let get = |key: &str| obj.and_then(|o| o.get(key));

    let chart_type = get("type")
        .and_then(|v| v.as_str())
        .and_then(ChartType::from_str)
        .unwrap_or(ChartType::Column);

    let labels: Vec<String> = get("labels")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(json_to_label).collect())
        .unwrap_or_default();

    let mut series: Vec<Series> = get("series")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    let values = s.get("values")?.as_array()?;
                    Some(Series {
                        name: s.get("name").map(json_to_label).unwrap_or_default(),
                        values: values.iter().map(to_number_or_null).collect(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    if series.len() > MAX_SERIES {
        // Fold the tail into one "기타" series rather than inventing a ninth hue.
        let rest = series.split_off(MAX_SERIES - 1);
        let length = rest.iter().map(|s| s.values.len()).max().unwrap_or(0);
        let summed: Vec<Option<f64>> = (0..length)
            .map(|i| {
                Some(
                    rest.iter()
                        .map(|s| s.values.get(i).copied().flatten().unwrap_or(0.0))
                        .sum(),
                )
            })
            .collect();
        series.push(Series {
            name: "기타".to_string(),
            values: summed,
        });
    }

    let opts = get("options").and_then(|v| v.as_object());
    let opt = |key: &str| opts.and_then(|o| o.get(key));

    ChartSpec {
        chart_type,
        title: get("title").map(json_to_label).unwrap_or_default(),
        labels,
        series,
        range: get("range").and_then(|v| v.as_str()).map(str::to_string),
        orientation: match get("orientation").and_then(|v| v.as_str()) {
            Some("rows") => Orientation::Rows,
            _ => Orientation::Columns,
        },
        options: ChartOptions {
            stacked: opt("stacked").and_then(|v| v.as_bool()).unwrap_or(false),
            legend: opt("legend").and_then(|v| v.as_bool()).unwrap_or(true),
            grid: opt("grid").and_then(|v| v.as_bool()).unwrap_or(true),
            value_labels: match opt("valueLabels").and_then(|v| v.as_str()) {
                Some("none") => ValueLabels::None,
                Some("ends") => ValueLabels::Ends,
                Some("all") => ValueLabels::All,
                _ => ValueLabels::Max,
            },
            y_title: opt("yTitle")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            number_format: opt("numberFormat")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
    }
}

/// `String(v)` for the label/name/title fields, which the JS coerced loosely.
fn json_to_label(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => {
            ai_formula::values::format_plain_number(n.as_f64().unwrap_or(f64::NAN))
        }
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn to_number_or_null(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) if s.is_empty() => None,
        serde_json::Value::Number(n) => n.as_f64().filter(|x| x.is_finite()),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok().filter(|x| x.is_finite()),
        serde_json::Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

static CHART_BLOCK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)```chart[^\n]*\n(.*?)```").unwrap());

/// Read a chart spec out of a ```chart fenced block.
pub fn parse_chart_block(md: &str) -> Option<ChartSpec> {
    let caps = CHART_BLOCK.captures(md)?;
    let json: serde_json::Value = serde_json::from_str(caps.get(1).unwrap().as_str()).ok()?;
    Some(normalize_spec(&json))
}

/// Write a chart spec back into a ```chart fenced block.
pub fn serialize_chart_block(spec: &ChartSpec) -> String {
    let json = serde_json::to_string_pretty(spec).unwrap_or_else(|_| "{}".to_string());
    format!("```chart\n{json}\n```")
}

/// What a sheet must expose for a chart to read a range out of it.
pub trait CellSource {
    fn cell(&self, reference: &str) -> Option<&Cell>;
}

#[derive(Default)]
pub struct ChartData {
    pub labels: Vec<String>,
    pub series: Vec<Series>,
}

/// Pull chart data out of a sheet range.
///
/// The first row (or column) is the label axis and the first cell of each series
/// is its name, which is what a user selecting a table expects.
pub fn chart_data_from_range(
    sheet: &dyn CellSource,
    range_spec: &str,
    orientation: Orientation,
) -> ChartData {
    let Some(r) = parse_range(range_spec) else {
        return ChartData::default();
    };

    let text = |col: usize, row: usize| {
        sheet
            .cell(&to_ref(col, row))
            .map(display_value)
            .unwrap_or_default()
    };
    let num = |col: usize, row: usize| -> Option<f64> {
        let cell = sheet.cell(&to_ref(col, row))?;
        if cell.t.as_deref() == Some("e") {
            return None;
        }
        cell.v.as_f64()
    };

    match orientation {
        Orientation::Columns => {
            // Columns are series: header row names them, first column labels the points.
            let labels = (r.start.row + 1..=r.end.row)
                .map(|row| text(r.start.col, row))
                .collect();
            let series = (r.start.col + 1..=r.end.col)
                .map(|col| {
                    let values = (r.start.row + 1..=r.end.row)
                        .map(|row| num(col, row))
                        .collect();
                    let name = {
                        let header = text(col, r.start.row);
                        if header.is_empty() {
                            to_ref(col, r.start.row)
                        } else {
                            header
                        }
                    };
                    Series { name, values }
                })
                .collect();
            ChartData { labels, series }
        }
        Orientation::Rows => {
            let labels = (r.start.col + 1..=r.end.col)
                .map(|col| text(col, r.start.row))
                .collect();
            let series = (r.start.row + 1..=r.end.row)
                .map(|row| {
                    let values = (r.start.col + 1..=r.end.col)
                        .map(|col| num(col, row))
                        .collect();
                    let name = {
                        let header = text(r.start.col, row);
                        if header.is_empty() {
                            to_ref(r.start.col, row)
                        } else {
                            header
                        }
                    };
                    Series { name, values }
                })
                .collect();
            ChartData { labels, series }
        }
    }
}

/// Resolve a spec's data, pulling from a sheet range when the spec has one.
///
/// If the range yields nothing usable — a bad address, a sheet that no longer
/// has those cells — any inline data on the spec is kept instead of rendering an
/// empty chart. Same principle as the rest of the format: a stale or hand-edited
/// file still shows what it can.
pub fn resolve_spec(spec: &ChartSpec, sheet: Option<&dyn CellSource>) -> ChartSpec {
    let (Some(range), Some(sheet)) = (spec.range.as_deref(), sheet) else {
        return spec.clone();
    };
    let data = chart_data_from_range(sheet, range, spec.orientation);
    let usable = data
        .series
        .iter()
        .any(|s| s.values.iter().any(|v| v.is_some()));
    if !usable {
        return spec.clone();
    }
    ChartSpec {
        labels: data.labels,
        series: data.series,
        ..spec.clone()
    }
}

/// How many points the chart plots.
pub fn point_count(spec: &ChartSpec) -> usize {
    if !spec.labels.is_empty() {
        spec.labels.len()
    } else {
        spec.series
            .iter()
            .map(|s| s.values.len())
            .max()
            .unwrap_or(0)
    }
}

fn series_name(spec: &ChartSpec, i: usize) -> String {
    let name = &spec.series[i].name;
    if name.is_empty() {
        format!("계열 {}", i + 1)
    } else {
        name.clone()
    }
}

/// The chart's numbers as a markdown table.
///
/// This is the accessibility relief the palette check requires — three
/// light-mode slots sit below 3:1 contrast — and it is also the reason a chart in
/// this format is legible to a model at all: `AI.md` carries the figures, not a
/// picture.
pub fn chart_to_markdown_table(spec: &ChartSpec) -> String {
    if spec.series.is_empty() {
        return "_차트 데이터가 없습니다_".to_string();
    }
    let mut header = vec!["구분".to_string()];
    for i in 0..spec.series.len() {
        header.push(series_name(spec, i));
    }
    let mut lines = vec![
        format!("| {} |", header.join(" | ")),
        format!("|{}|", vec!["---"; header.len()].join("|")),
    ];

    for i in 0..point_count(spec) {
        let label = spec.labels.get(i).filter(|l| !l.is_empty()).cloned();
        let mut cells = vec![label.unwrap_or_else(|| (i + 1).to_string())];
        for s in &spec.series {
            cells.push(match s.values.get(i).copied().flatten() {
                None => String::new(),
                Some(v) => ai_formula::values::format_plain_number(v),
            });
        }
        lines.push(format!("| {} |", cells.join(" | ")));
    }
    lines.join("\n")
}

/// One-line natural-language description, for the digest.
pub fn describe_chart(spec: &ChartSpec) -> String {
    let stacked = if spec.options.stacked { " 누적" } else { "" };
    let names: Vec<String> = (0..spec.series.len())
        .map(|i| series_name(spec, i))
        .collect();
    let title = if spec.title.is_empty() {
        String::new()
    } else {
        format!(" \"{}\"", spec.title)
    };
    let source = match &spec.range {
        Some(r) => format!(" (범위 `{r}`)"),
        None => String::new(),
    };
    format!(
        "{}{stacked} 차트{title} — 계열 {}개({}), 항목 {}개{source}",
        spec.chart_type.label(),
        spec.series.len(),
        names.join(", "),
        point_count(spec)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use serde_json::json;

    struct Sheet(IndexMap<String, Cell>);

    impl CellSource for Sheet {
        fn cell(&self, reference: &str) -> Option<&Cell> {
            self.0.get(reference)
        }
    }

    fn sheet(pairs: &[(&str, serde_json::Value)]) -> Sheet {
        Sheet(
            pairs
                .iter()
                .map(|(k, v)| {
                    let t = if v.is_number() { "n" } else { "s" };
                    (
                        k.to_string(),
                        Cell {
                            v: v.clone(),
                            t: Some(t.into()),
                            ..Cell::default()
                        },
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn defaults_fill_in_for_a_bare_spec() {
        let spec = normalize_spec(&json!({}));
        assert_eq!(spec.chart_type, ChartType::Column);
        assert!(spec.options.legend);
        assert_eq!(spec.options.value_labels, ValueLabels::Max);
        assert_eq!(spec.orientation, Orientation::Columns);
    }

    #[test]
    fn an_unknown_type_falls_back_to_column() {
        assert_eq!(
            normalize_spec(&json!({"type": "radar"})).chart_type,
            ChartType::Column
        );
    }

    #[test]
    fn a_ninth_series_folds_into_기타() {
        let series: Vec<serde_json::Value> = (0..10)
            .map(|i| json!({"name": format!("S{i}"), "values": [1, 2]}))
            .collect();
        let spec = normalize_spec(&json!({"series": series}));
        assert_eq!(spec.series.len(), MAX_SERIES);
        assert_eq!(spec.series[7].name, "기타");
        // Three folded series of 1 and 2 sum to 3 and 6.
        assert_eq!(spec.series[7].values, [Some(3.0), Some(6.0)]);
    }

    #[test]
    fn blanks_stay_gaps_not_zeros() {
        let spec = normalize_spec(&json!({"series": [{"values": [1, null, "", "x", 3]}]}));
        assert_eq!(
            spec.series[0].values,
            [Some(1.0), None, None, None, Some(3.0)]
        );
    }

    #[test]
    fn a_chart_block_round_trips() {
        let spec = normalize_spec(&json!({
            "type": "line", "title": "매출", "labels": ["1분기"],
            "series": [{"name": "2026", "values": [120]}],
        }));
        let block = serialize_chart_block(&spec);
        assert!(block.starts_with("```chart\n"));
        assert_eq!(parse_chart_block(&block), Some(spec));
    }

    #[test]
    fn a_malformed_chart_block_is_ignored() {
        assert_eq!(parse_chart_block("```chart\nnot json\n```"), None);
        assert_eq!(parse_chart_block("# 그냥 제목"), None);
    }

    #[test]
    fn a_range_becomes_labels_and_series() {
        let s = sheet(&[
            ("A1", json!("구분")),
            ("B1", json!("2025")),
            ("C1", json!("2026")),
            ("A2", json!("1분기")),
            ("B2", json!(98)),
            ("C2", json!(120)),
            ("A3", json!("2분기")),
            ("B3", json!(101)),
            ("C3", json!(133)),
        ]);
        let data = chart_data_from_range(&s, "A1:C3", Orientation::Columns);
        assert_eq!(data.labels, ["1분기", "2분기"]);
        assert_eq!(data.series.len(), 2);
        assert_eq!(data.series[0].name, "2025");
        assert_eq!(data.series[0].values, [Some(98.0), Some(101.0)]);
        assert_eq!(data.series[1].values, [Some(120.0), Some(133.0)]);
    }

    #[test]
    fn row_orientation_transposes() {
        let s = sheet(&[
            ("A1", json!("구분")),
            ("B1", json!("1분기")),
            ("C1", json!("2분기")),
            ("A2", json!("2025")),
            ("B2", json!(98)),
            ("C2", json!(101)),
        ]);
        let data = chart_data_from_range(&s, "A1:C2", Orientation::Rows);
        assert_eq!(data.labels, ["1분기", "2분기"]);
        assert_eq!(data.series[0].name, "2025");
        assert_eq!(data.series[0].values, [Some(98.0), Some(101.0)]);
    }

    #[test]
    fn a_broken_range_keeps_the_inline_data() {
        let spec = normalize_spec(&json!({
            "range": "ZZZ999:A1",
            "labels": ["a"],
            "series": [{"name": "s", "values": [7]}],
        }));
        let s = sheet(&[]);
        let resolved = resolve_spec(&spec, Some(&s));
        assert_eq!(resolved.series[0].values, [Some(7.0)]);
        assert_eq!(resolved.labels, ["a"]);
    }

    #[test]
    fn the_markdown_table_carries_the_numbers() {
        let spec = normalize_spec(&json!({
            "labels": ["1분기", "2분기"],
            "series": [{"name": "2025", "values": [98, null]}, {"name": "2026", "values": [120, 133]}],
        }));
        assert_eq!(
            chart_to_markdown_table(&spec),
            "| 구분 | 2025 | 2026 |\n|---|---|---|\n| 1분기 | 98 | 120 |\n| 2분기 |  | 133 |"
        );
    }

    #[test]
    fn an_empty_chart_says_so() {
        assert_eq!(
            chart_to_markdown_table(&ChartSpec::default()),
            "_차트 데이터가 없습니다_"
        );
    }

    #[test]
    fn descriptions_read_as_a_sentence() {
        let spec = normalize_spec(&json!({
            "type": "column", "title": "분기별 매출", "range": "A1:D5",
            "labels": ["1분기", "2분기", "3분기", "4분기"],
            "series": [{"name": "2025", "values": [1, 2, 3, 4]}, {"name": "2026", "values": [1, 2, 3, 4]}],
        }));
        assert_eq!(
            describe_chart(&spec),
            "세로 막대 차트 \"분기별 매출\" — 계열 2개(2025, 2026), 항목 4개 (범위 `A1:D5`)"
        );
    }
}
