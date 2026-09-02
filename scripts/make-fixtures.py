#!/usr/bin/env python3
"""Write Office files with *other* implementations, for the importers to read.

A round trip through this project's own writer cannot catch what it never
emits — placeholder geometry inheritance, theme colours, list styles, Word's
vertical merges. These fixtures come from openpyxl, python-pptx and python-docx,
so they carry the shapes a real file has.

    python3 scripts/make-fixtures.py out/
"""
import datetime
import pathlib
import sys


def workbook(path):
    import openpyxl
    from openpyxl.styles import Alignment, Border, Font, PatternFill, Side
    from openpyxl.utils import get_column_letter
    from openpyxl.workbook.defined_name import DefinedName

    wb = openpyxl.Workbook()
    ws = wb.active
    ws.title = "예산"
    for i, head in enumerate(["항목", "10월", "11월", "12월", "합계", "비중"], 1):
        cell = ws.cell(1, i, head)
        cell.font = Font(bold=True, color="FFFFFFFF")
        cell.fill = PatternFill("solid", fgColor="FF2A78D6")
        cell.alignment = Alignment(horizontal="center")

    rows = [("인건비", 480000000, 495000000, 495000000),
            ("마케팅", 210000000, 260000000, 320000000),
            ("인프라", 88000000, 92000000, 96000000)]
    for r, (label, *months) in enumerate(rows, start=2):
        ws.cell(r, 1, label)
        for col, value in zip((2, 3, 4), months):
            cell = ws.cell(r, col, value)
            cell.number_format = "₩#,##0"
        ws.cell(r, 5, f"=SUM(B{r}:D{r})").number_format = "₩#,##0"
        ws.cell(r, 6, f"=E{r}/$E$5").number_format = "0.0%"

    total = len(rows) + 2
    ws.cell(total, 1, "합계").font = Font(bold=True)
    for col in range(2, 7):
        letter = get_column_letter(col)
        cell = ws.cell(total, col, f"=SUM({letter}2:{letter}{total - 1})")
        cell.number_format = "0.0%" if col == 6 else "₩#,##0"
        cell.font = Font(bold=True)
        cell.border = Border(top=Side(style="thin"), bottom=Side(style="double"))

    ws.freeze_panes = "B2"
    ws.column_dimensions["A"].width = 22
    ws.row_dimensions[1].height = 30
    ws.merge_cells("A7:C7")
    ws.cell(7, 1, "비고: 4분기 전망 별도")
    wb.defined_names.add(DefinedName("예산표", attr_text="'예산'!$A$1:$F$5"))

    other = wb.create_sheet("가정")
    other["A1"], other["B1"] = "가정", "값"
    other["A2"], other["B2"] = "환율", 9.2
    other["B2"].number_format = "#,##0.00"
    other["A3"], other["B3"] = "인상률", 0.031
    other["B3"].number_format = "0.0%"
    other["A4"], other["B4"] = "기준일", datetime.date(2026, 10, 1)
    other["B4"].number_format = "yyyy-mm-dd"
    other["A5"], other["B5"] = "오류", "=1/0"
    wb.save(path)


def deck(path):
    from pptx import Presentation
    from pptx.chart.data import CategoryChartData
    from pptx.dml.color import RGBColor
    from pptx.enum.chart import XL_CHART_TYPE
    from pptx.enum.shapes import MSO_SHAPE
    from pptx.enum.text import MSO_ANCHOR, PP_ALIGN
    from pptx.util import Emu, Inches, Pt

    prs = Presentation()
    prs.slide_width, prs.slide_height = Inches(13.333), Inches(7.5)

    # Placeholders with no geometry of their own — the common case.
    s = prs.slides.add_slide(prs.slide_layouts[0])
    s.shapes.title.text = "2026 3분기 사업 리뷰"
    s.placeholders[1].text = "매출 성장 · 신규 시장 진입"
    s.notes_slide.notes_text_frame.text = "인사 후 바로 핵심 숫자로."

    # Bullets from the master, emphasis, a text box that must not get bullets.
    s = prs.slides.add_slide(prs.slide_layouts[1])
    s.shapes.title.text = "핵심 성과"
    frame = s.placeholders[1].text_frame
    frame.text = "매출 142억"
    frame.paragraphs[0].runs[0].font.bold = True
    para = frame.add_paragraph()
    para.text = "전년 동기 대비 +24%"
    para.level = 1
    box = s.shapes.add_textbox(Inches(1), Inches(5.5), Inches(6), Inches(0.8))
    para = box.text_frame.paragraphs[0]
    para.alignment = PP_ALIGN.CENTER
    run = para.add_run()
    run.text = "출처: 재무팀"
    run.font.size = Pt(14)
    run.font.color.rgb = RGBColor(0x6B, 0x72, 0x80)

    # Shapes: explicit colours, a themed one, a transparent one, rotation.
    s = prs.slides.add_slide(prs.slide_layouts[6])
    shape = s.shapes.add_shape(MSO_SHAPE.DIAMOND, Inches(1), Inches(1), Inches(3), Inches(2))
    shape.text_frame.text = "승인?"
    shape.fill.solid()
    shape.fill.fore_color.rgb = RGBColor(0xDB, 0xEA, 0xFE)
    shape.line.color.rgb = RGBColor(0x2A, 0x78, 0xD6)
    shape.line.width = Pt(2)
    shape.rotation = 15.0
    arrow = s.shapes.add_shape(MSO_SHAPE.RIGHT_ARROW, Inches(5), Inches(1.5), Inches(2), Inches(1))
    arrow.fill.background()
    arrow.line.dash_style = 4
    s.shapes.add_shape(MSO_SHAPE.STAR_5_POINT, Inches(8), Inches(1), Inches(2), Inches(2))

    # A table with a merge and per-cell alignment.
    s = prs.slides.add_slide(prs.slide_layouts[6])
    table = s.shapes.add_table(3, 3, Inches(1), Inches(1.5), Inches(8), Inches(2)).table
    for i, width in enumerate((3, 2.5, 2.5)):
        table.columns[i].width = Emu(int(Inches(width)))
    data = [["항목", "상반기", ""], ["제품 A", "1,200", "1,350"], ["제품 B", "980", "1,120"]]
    for r in range(3):
        for c in range(3):
            cell = table.cell(r, c)
            cell.text = data[r][c]
            cell.text_frame.paragraphs[0].alignment = PP_ALIGN.RIGHT if c else PP_ALIGN.LEFT
            cell.vertical_anchor = MSO_ANCHOR.MIDDLE
    table.cell(0, 1).merge(table.cell(0, 2))

    # A native chart.
    s = prs.slides.add_slide(prs.slide_layouts[6])
    chart_data = CategoryChartData()
    chart_data.categories = ["1분기", "2분기", "3분기", "4분기"]
    chart_data.add_series("2025", (98, 101, 96, 110))
    chart_data.add_series("2026", (120, 133, 142, 155))
    frame = s.shapes.add_chart(XL_CHART_TYPE.COLUMN_CLUSTERED, Inches(1), Inches(1),
                               Inches(8), Inches(4.5), chart_data)
    frame.chart.has_title = True
    frame.chart.chart_title.text_frame.text = "분기별 매출"

    prs.save(path)


def document(path):
    import docx
    from docx.enum.text import WD_ALIGN_PARAGRAPH
    from docx.shared import Inches, Pt, RGBColor

    d = docx.Document()
    section = d.sections[0]
    section.page_width, section.page_height = Inches(8.27), Inches(11.69)
    section.left_margin = section.right_margin = Inches(1)

    d.add_heading("AI Studio 저장 포맷 제안서", level=1)
    para = d.add_paragraph("기존 오피스 포맷은 zip 안에 ")
    para.add_run("수십 개의 XML 파트").bold = True
    para.add_run("를 담는다. ")
    para.add_run("기울임").italic = True

    d.add_heading("설계 원칙", level=2)
    for text in ["텍스트가 런 단위로 조각난다", "위치가 EMU 좌표로만 존재한다"]:
        d.add_paragraph(text, style="List Bullet")
    for text in ["첫째 단계", "둘째 단계"]:
        d.add_paragraph(text, style="List Number")
    d.add_paragraph("이 제안서는 폴더 기반 포맷을 정의한다.", style="Quote")

    para = d.add_paragraph("가운데 정렬 문단")
    para.alignment = WD_ALIGN_PARAGRAPH.CENTER
    para.paragraph_format.left_indent = Pt(18)
    for run in para.runs:
        run.font.size = Pt(14)
        run.font.color.rgb = RGBColor(0x6B, 0x72, 0x80)

    d.add_heading("비교", level=2)
    table = d.add_table(rows=3, cols=3)
    table.style = "Table Grid"
    rows = [["구분", "OOXML", "AI Studio"],
            ["컨테이너", "zip + XML", "일반 폴더"],
            ["버전 관리", "바이너리 diff", "텍스트 diff"]]
    for r, row in enumerate(rows):
        for c, text in enumerate(row):
            cell = table.cell(r, c)
            cell.text = text
            if r == 0:
                cell.paragraphs[0].runs[0].bold = True
    d.save(path)



def report(path):
    """A workbook shaped like a real report: conditional formatting, a chart, a
    summary sheet that reaches across, and the number formats an accountant uses.

    Every one of these is something an importer can silently lose."""
    import openpyxl
    from openpyxl.chart import BarChart, Reference
    from openpyxl.formatting.rule import CellIsRule, ColorScaleRule, Rule
    from openpyxl.styles import Alignment, Font, PatternFill
    from openpyxl.styles.differential import DifferentialStyle
    from openpyxl.workbook.defined_name import DefinedName

    wb = openpyxl.Workbook()
    ws = wb.active
    ws.title = "실적"
    heads = ["부문", "목표", "실적", "달성률", "증감", "점수"]
    for i, head in enumerate(heads, 1):
        cell = ws.cell(1, i, head)
        cell.font = Font(bold=True, size=12)
        cell.fill = PatternFill("solid", fgColor="FFF3F2F1")
        cell.alignment = Alignment(horizontal="center")
    rows = [
        ("서울", 1200000000, 1440000000, 120, 90),
        ("부산", 900000000, 855000000, -45, 60),
        ("대구", 600000000, 588000000, -12, 75),
        ("광주", 400000000, 463000000, 63, 95),
        ("대전", 300000000, 292000000, -8, 40),
    ]
    for r, (name, goal, actual, delta, score) in enumerate(rows, start=2):
        ws.cell(r, 1, name)
        ws.cell(r, 2, goal).number_format = "₩#,##0"
        ws.cell(r, 3, actual).number_format = "₩#,##0"
        ws.cell(r, 4, f"=C{r}/B{r}").number_format = "0.0%"
        ws.cell(r, 5, delta).number_format = "#,##0_);[Red](#,##0)"
        ws.cell(r, 6, score)
    ws.cell(7, 1, "합계").font = Font(bold=True)
    for col in ("B", "C"):
        # A shared formula in Excel's own shape: written once, filled down.
        ws[f"{col}7"] = f"=SUM({col}2:{col}6)"
        ws[f"{col}7"].number_format = "₩#,##0"
        ws[f"{col}7"].font = Font(bold=True)
    ws["D7"] = "=C7/B7"
    ws["D7"].number_format = "0.0%"
    ws.freeze_panes = "B2"
    ws.column_dimensions["A"].width = 12
    wb.defined_names.add(DefinedName("실적범위", attr_text="실적!$C$2:$C$6"))

    # Red on a negative, a heat map on the score, and a formula rule that bolds
    # the name of a division that beat its goal.
    ws.conditional_formatting.add(
        "E2:E6",
        CellIsRule(operator="lessThan", formula=["0"],
                   font=Font(color="9C0006"),
                   fill=PatternFill(start_color="FFC7CE", end_color="FFC7CE")))
    ws.conditional_formatting.add(
        "F2:F6",
        ColorScaleRule(start_type="min", start_color="F8696B",
                       mid_type="percentile", mid_value=50, mid_color="FFEB84",
                       end_type="max", end_color="63BE7B"))
    ws.conditional_formatting.add(
        "A2:A6",
        Rule(type="expression", dxf=DifferentialStyle(font=Font(bold=True)),
             formula=["$C2>$B2"]))

    chart = BarChart()
    chart.title = "부문별 실적"
    chart.add_data(Reference(ws, min_col=3, min_row=1, max_row=6), titles_from_data=True)
    chart.set_categories(Reference(ws, min_col=1, min_row=2, max_row=6))
    ws.add_chart(chart, "H2")

    # A summary sheet is mostly cross-sheet references and the newer functions.
    summary = wb.create_sheet("요약")
    summary["A1"] = "지표"
    summary["B1"] = "값"
    checks = [
        ("총 실적", "=SUM(실적!C2:C6)", "₩#,##0"),
        ("목표 달성", "=SUMIFS(실적!C2:C6,실적!E2:E6,\">0\")", "₩#,##0"),
        ("미달 부문 수", "=COUNTIFS(실적!E2:E6,\"<0\")", "0"),
        ("최고 점수", "=MAX(실적!F2:F6)", "0"),
        ("서울 실적", "=XLOOKUP(\"서울\",실적!A2:A6,실적!C2:C6)", "₩#,##0"),
        ("점수 표준편차", "=STDEV.S(실적!F2:F6)", "0.00"),
        ("이름 범위 합", "=SUM(실적범위)", "₩#,##0"),
        ("월 상환액", "=PMT(0.05/12,360,300000000)", "₩#,##0"),
        ("다음 달 말일", "=EOMONTH(TODAY(),1)", "yyyy-mm-dd"),
        ("영업일 수", "=NETWORKDAYS(DATE(2026,9,1),DATE(2026,9,30))", "0\"일\""),
    ]
    for r, (label, formula, fmt) in enumerate(checks, start=2):
        summary.cell(r, 1, label)
        cell = summary.cell(r, 2, formula)
        cell.number_format = fmt
    summary.column_dimensions["A"].width = 16
    summary.column_dimensions["B"].width = 18
    wb.save(path)


def paper(path):
    """A document with the furniture Word documents have: a header and footer with
    a page-number field, footnotes, two columns, and a landscape page."""
    from docx import Document
    from docx.enum.section import WD_ORIENT, WD_SECTION
    from docx.enum.text import WD_ALIGN_PARAGRAPH
    from docx.oxml import OxmlElement
    from docx.oxml.ns import qn
    from docx.shared import Mm, Pt

    d = Document()
    section = d.sections[0]
    header = section.header.paragraphs[0]
    header.text = "AI Studio · 2026 3분기 리뷰"
    header.alignment = WD_ALIGN_PARAGRAPH.CENTER

    footer = section.footer.paragraphs[0]
    footer.alignment = WD_ALIGN_PARAGRAPH.CENTER
    footer.add_run("기밀\t")
    field = footer.add_run()
    for kind, text in (("begin", None), (None, " PAGE "), ("end", None)):
        if kind:
            element = OxmlElement("w:fldChar")
            element.set(qn("w:fldCharType"), kind)
        else:
            element = OxmlElement("w:instrText")
            element.text = text
        field._r.append(element)
    footer.add_run(" / ")
    total = footer.add_run()
    for kind, text in (("begin", None), (None, " NUMPAGES "), ("end", None)):
        if kind:
            element = OxmlElement("w:fldChar")
            element.set(qn("w:fldCharType"), kind)
        else:
            element = OxmlElement("w:instrText")
            element.text = text
        total._r.append(element)

    d.add_heading("분기 리뷰 보고서", level=1)
    body = d.add_paragraph("매출은 전년 동기 대비 24% 늘었다")
    reference = body.add_run()
    note = OxmlElement("w:footnoteReference")
    note.set(qn("w:id"), "2")
    reference._r.append(note)
    body.add_run(". 신규 시장 진입이 대부분을 설명한다.")
    d.add_heading("세부 지표", level=2)
    for line in ["서울·부산 두 지역이 성장을 이끌었다", "인건비는 계획 범위 안에 있다"]:
        d.add_paragraph(line, style="List Bullet")
    quote = d.add_paragraph("목표 대비 106%를 달성했다.")
    quote.style = d.styles["Quote"] if "Quote" in [s.name for s in d.styles] else quote.style
    small = d.add_paragraph("표는 다음 장에 있다.")
    small.runs[0].font.size = Pt(9)

    # A two-column section, then a landscape one — both things a reader notices.
    columns = d.add_section(WD_SECTION.CONTINUOUS)
    cols = columns._sectPr.xpath("./w:cols")[0]
    cols.set(qn("w:num"), "2")
    cols.set(qn("w:space"), "720")
    d.add_heading("두 단 구성", level=2)
    for i in range(4):
        d.add_paragraph(
            f"두 단으로 흐르는 문단 {i + 1}. 왼쪽 단이 끝나면 오른쪽 단으로 이어지고, "
            "오른쪽 단이 끝나면 다음 페이지로 넘어간다."
        )

    wide = d.add_section(WD_SECTION.NEW_PAGE)
    wide.orientation = WD_ORIENT.LANDSCAPE
    wide.page_width, wide.page_height = Mm(297), Mm(210)
    single = wide._sectPr.xpath("./w:cols")[0]
    single.set(qn("w:num"), "1")
    d.add_heading("가로 페이지", level=2)
    table = d.add_table(rows=3, cols=3)
    table.style = "Table Grid"
    for r, row in enumerate([("지역", "목표", "실적"), ("서울", "12억", "14.4억"), ("부산", "9억", "8.55억")]):
        for c, text in enumerate(row):
            cell = table.cell(r, c)
            cell.text = text
            if r == 0:
                cell.paragraphs[0].runs[0].bold = True
    d.save(path)

    # python-docx has no footnote API, so the part is written into the saved file.
    import zipfile
    notes = """<?xml version="1.0" encoding="UTF-8"?>
<w:footnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:footnote w:id="0" w:type="separator"><w:p/></w:footnote>
  <w:footnote w:id="2"><w:p><w:r><w:t>재무팀 2026년 9월 확정 자료.</w:t></w:r></w:p></w:footnote>
</w:footnotes>"""
    source = zipfile.ZipFile(path)
    parts = [(n, source.read(n)) for n in source.namelist()]
    source.close()
    out = zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED)
    for name, data in parts:
        if name == "[Content_Types].xml":
            data = data.decode().replace(
                "</Types>",
                '<Override PartName="/word/footnotes.xml" ContentType='
                '"application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/></Types>',
            ).encode()
        if name == "word/_rels/document.xml.rels":
            data = data.decode().replace(
                "</Relationships>",
                '<Relationship Id="rIdFootnotes" Type="http://schemas.openxmlformats.org/'
                'officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/></Relationships>',
            ).encode()
        out.writestr(name, data)
    out.writestr("word/footnotes.xml", notes)
    out.close()


def themed(path):
    """A deck whose design lives on the layout, as every template's does: a
    banner, a background, a footer and a slide number."""
    deck(path)

    import re
    import zipfile

    banner = (
        '<p:sp><p:nvSpPr><p:cNvPr id="90" name="띠"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>'
        '<p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="12192000" cy="457200"/></a:xfrm>'
        '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>'
        '<a:solidFill><a:srgbClr val="1F3864"/></a:solidFill></p:spPr>'
        "<p:txBody><a:bodyPr/><a:p/></p:txBody></p:sp>"
    )
    footer = (
        '<p:sp><p:nvSpPr><p:cNvPr id="91" name="바닥글"/><p:cNvSpPr/>'
        '<p:nvPr><p:ph type="ftr" idx="11"/></p:nvPr></p:nvSpPr>'
        '<p:spPr><a:xfrm><a:off x="457200" y="6400800"/><a:ext cx="4114800" cy="365125"/></a:xfrm></p:spPr>'
        "<p:txBody><a:bodyPr/><a:p><a:r><a:t>AI Studio · 대외비</a:t></a:r></a:p></p:txBody></p:sp>"
    )
    number = (
        '<p:sp><p:nvSpPr><p:cNvPr id="92" name="슬라이드 번호"/><p:cNvSpPr/>'
        '<p:nvPr><p:ph type="sldNum" idx="12"/></p:nvPr></p:nvSpPr>'
        '<p:spPr><a:xfrm><a:off x="11201400" y="6400800"/><a:ext cx="533400" cy="365125"/></a:xfrm></p:spPr>'
        "<p:txBody><a:bodyPr/><a:p><a:fld id=\"{A1}\" type=\"slidenum\">"
        "<a:t>\u2039#\u203a</a:t></a:fld></a:p></p:txBody></p:sp>"
    )
    background = (
        "<p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"F5F7FB\"/></a:solidFill>"
        "<a:effectLst/></p:bgPr></p:bg>"
    )

    source = zipfile.ZipFile(path)
    parts = [(n, source.read(n)) for n in source.namelist()]
    source.close()
    out = zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED)
    for name, data in parts:
        if name.startswith("ppt/slideLayouts/slideLayout") and name.endswith(".xml"):
            text = data.decode()
            text = re.sub(r"(<p:cSld[^>]*>)", r"\1" + background, text, count=1)
            text = text.replace("</p:spTree>", banner + footer + number + "</p:spTree>", 1)
            data = text.encode()
        out.writestr(name, data)
    out.close()



def standard(path):
    """A 4:3 deck — the size a projector deck is still made at, and the one an
    importer that assumes 16:9 puts every shape in the wrong place on."""
    from pptx import Presentation
    from pptx.util import Emu, Inches, Pt

    prs = Presentation()
    prs.slide_width, prs.slide_height = Emu(9144000), Emu(6858000)  # 10 x 7.5in

    title = prs.slides.add_slide(prs.slide_layouts[0])
    title.shapes.title.text = "4:3 표준 비율 덱"
    title.placeholders[1].text = "프로젝터용으로 만든 파일"

    content = prs.slides.add_slide(prs.slide_layouts[1])
    content.shapes.title.text = "화면비가 다른 파일"
    body = content.placeholders[1].text_frame
    body.text = "슬라이드는 960 x 720px으로 열립니다"
    for line in ["도형은 자기 자리에 그대로", "썸네일도 4:3으로"]:
        body.add_paragraph().text = line

    # A shape hard against the bottom-right corner: on a widescreen canvas it
    # would float in the middle of the slide.
    from pptx.enum.shapes import MSO_SHAPE
    from pptx.dml.color import RGBColor

    corner = content.shapes.add_shape(
        MSO_SHAPE.ROUNDED_RECTANGLE, Inches(7.0), Inches(6.2), Inches(2.6), Inches(1.0)
    )
    corner.fill.solid()
    corner.fill.fore_color.rgb = RGBColor(0x1F, 0x38, 0x64)
    corner.text_frame.text = "우측 하단"
    corner.text_frame.paragraphs[0].runs[0].font.size = Pt(14)
    prs.save(path)


def portrait(path):
    """An A4-portrait deck, the shape a printed handout template has."""
    from pptx import Presentation
    from pptx.util import Emu, Inches, Pt

    prs = Presentation()
    # 7.5 x 10.58in — A4 the tall way, as PowerPoint's page setup writes it.
    prs.slide_width, prs.slide_height = Emu(7099300), Emu(10058400)

    slide = prs.slides.add_slide(prs.slide_layouts[5])
    slide.shapes.title.text = "세로 A4 덱"
    box = slide.shapes.add_textbox(Inches(0.8), Inches(2.0), Inches(6.0), Inches(6.0))
    frame = box.text_frame
    frame.word_wrap = True
    frame.text = "인쇄용 유인물은 세로로 만들어집니다."
    for line in [
        "캔버스는 745 x 1056px",
        "창에 맞춤은 높이를 기준으로 잡습니다",
        "내보내면 sldSz도 세로로 나갑니다",
    ]:
        paragraph = frame.add_paragraph()
        paragraph.text = line
        paragraph.font.size = Pt(16)
    prs.save(path)


def main():
    out = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "office-fixtures")
    out.mkdir(parents=True, exist_ok=True)
    for name, build in (
        ("excel.xlsx", workbook),
        ("deck.pptx", deck),
        ("doc.docx", document),
        # The files the conversions are checked against: things this format has no
        # exact equivalent for.
        ("report.xlsx", report),
        ("paper.docx", paper),
        ("themed.pptx", themed),
        # Decks that are not 16:9 — the sizes people actually make.
        ("standard-4x3.pptx", standard),
        ("portrait-a4.pptx", portrait),
    ):
        target = out / name
        build(target)
        print(f"  {target}  {target.stat().st_size} bytes")


if __name__ == "__main__":
    main()
