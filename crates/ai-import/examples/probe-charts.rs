//! What charts an imported workbook kept.

fn main() {
    let path = std::env::args().nth(1).expect("usage: probe-charts <xlsx>");
    let bytes = std::fs::read(&path).expect("readable file");
    let package = ai_import::ooxml::Package::open(&bytes).expect("a package");
    for sheet in ai_import::xlsx::read(&package).expect("readable workbook") {
        println!("── 시트 {} · 차트 {}개", sheet.name, sheet.charts.len());
        for chart in &sheet.charts {
            println!(
                "   {:?} “{}”  {},{} {}x{}  계열 {}개, 범위 {:?}",
                chart.spec.chart_type,
                chart.spec.title,
                chart.x,
                chart.y,
                chart.w,
                chart.h,
                chart.spec.series.len(),
                chart.spec.range
            );
        }
    }
}
