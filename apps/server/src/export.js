import path from 'node:path';
import { promises as fs } from 'node:fs';

import {
  PAGE_SIZES, usedRange,
  parseChartBlock, resolveChartSpec, chartToMarkdownTable, describeChart,
  CHART_TYPE_LABELS, CHART_PALETTE,
} from '@ai-studio/format';
import { toRef, indexToCol, displayValue, recalcSheet, fromSerial } from '@ai-studio/formula';
import { parseMarkdown, runsToText } from './mdruns.js';

/** px -> inches at the 96dpi the canvas coordinates assume. */
const IN = (px) => px / 96;
/** px -> points, for docx font sizes and spacing. */
const PT = (px) => px * 0.75;
/** px -> twips (1/20 pt), the unit docx uses for indents and margins. */
const TWIP = (px) => Math.round(px * 15);

const HEADING_SIZE = [40, 32, 26, 22, 19, 17];

/* ------------------------------------------------------------------- pptx */

/**
 * Export a deck to .pptx.
 *
 * Block geometry maps straight across — the canvas is already a 16:9 pixel grid,
 * so dividing by 96 gives inches. Text keeps its markdown-derived structure as
 * pptxgenjs rich-text runs, so bold stays bold and bullets stay bullets rather
 * than arriving as one flat string.
 */
export async function exportDeckPptx(project) {
  const { default: PptxGenJS } = await import('pptxgenjs');
  const pptx = new PptxGenJS();

  const first = project.slides[0]?.canvas ?? { w: 1280, h: 720 };
  pptx.defineLayout({ name: 'AISTUDIO', width: IN(first.w), height: IN(first.h) });
  pptx.layout = 'AISTUDIO';
  pptx.title = project.manifest?.title ?? 'AI Studio Deck';

  for (const slide of project.slides) {
    const s = pptx.addSlide();
    const canvas = slide.canvas ?? first;
    if (canvas.bg && canvas.bg !== '#ffffff') s.background = { color: hex(canvas.bg) };

    if (slide.notes?.trim()) s.addNotes(slide.notes);

    const ordered = [...slide.blocks].sort((a, b) => (a.z ?? 0) - (b.z ?? 0));
    for (const block of ordered) {
      const box = {
        x: IN(block.x),
        y: IN(block.y),
        w: IN(block.w),
        h: IN(block.h),
      };

      if (block.kind === 'shape') {
        s.addShape(pptx.ShapeType.roundRect, {
          ...box,
          fill: { color: hex(block.style?.fill ?? '#e5e7eb') },
          line: { type: 'none' },
          rectRadius: 0.06,
        });
      }

      if (block.kind === 'image') {
        const src = imageSource(block.md);
        const file = src ? await resolveAsset(project.dir, src) : null;
        if (file) {
          s.addImage({ ...box, path: file, sizing: { type: 'contain', w: box.w, h: box.h } });
          continue;
        }
        // Unresolvable image: keep the alt text so the slide is not silently empty.
        s.addText(imageAlt(block.md) || '(이미지를 찾을 수 없음)', {
          ...box,
          fontSize: 12,
          color: '9CA3AF',
          italic: true,
          align: 'center',
          valign: 'middle',
        });
        continue;
      }

      if (block.kind === 'chart') {
        const spec = parseChartBlock(block.md);
        if (spec && spec.series.length) {
          addPptxChart(pptx, s, spec, box);
          continue;
        }
        s.addText('(차트 정의를 읽을 수 없음)', {
          ...box, fontSize: 12, color: '9CA3AF', italic: true, align: 'center', valign: 'middle',
        });
        continue;
      }

      if (block.kind === 'table') {
        const table = tableRows(block.md);
        if (table.length) {
          s.addTable(
            table.map((row, r) =>
              row.map((cell) => ({
                text: cell,
                options: { bold: r === 0, fill: r === 0 ? 'F1F5F9' : undefined },
              }))
            ),
            { ...box, fontSize: Math.max(8, PT(block.style?.fontSize ?? 16)), border: { pt: 0.5, color: 'D1D5DB' } }
          );
          continue;
        }
      }

      const runs = pptxRuns(block.md, block.style ?? {});
      if (!runs.length) continue;
      s.addText(runs, {
        ...box,
        align: block.style?.align ?? 'left',
        valign: block.style?.valign === 'middle' ? 'middle' : block.style?.valign === 'bottom' ? 'bottom' : 'top',
        color: hex(block.style?.color ?? '#1f2937'),
        fontSize: PT(block.style?.fontSize ?? 20),
        lineSpacingMultiple: block.style?.lineHeight ?? 1.3,
        margin: 2,
      });
    }
  }

  const data = await pptx.write({ outputType: 'nodebuffer' });
  return Buffer.isBuffer(data) ? data : Buffer.from(data);
}

/**
 * Add a chart PowerPoint can edit, rather than a picture of one.
 *
 * pptxgenjs writes a real chart part, so the recipient can retype a number and the
 * chart updates — which is the whole reason to export to .pptx instead of a PNG.
 */
function addPptxChart(pptx, slide, spec, box) {
  const TYPES = {
    column: { type: pptx.ChartType.bar, barDir: 'col' },
    bar: { type: pptx.ChartType.bar, barDir: 'bar' },
    line: { type: pptx.ChartType.line },
    area: { type: pptx.ChartType.area },
    pie: { type: pptx.ChartType.pie },
    donut: { type: pptx.ChartType.doughnut },
  };
  const mapped = TYPES[spec.type] ?? TYPES.column;
  const labels = spec.labels.length
    ? spec.labels
    : spec.series[0].values.map((_, i) => String(i + 1));

  const data = spec.series.map((series, i) => ({
    name: series.name || `계열 ${i + 1}`,
    labels,
    // PowerPoint has no notion of a gap, so a missing point becomes zero.
    values: series.values.map((v) => (v === null || v === undefined ? 0 : v)),
  }));

  slide.addChart(mapped.type, data, {
    ...box,
    ...(mapped.barDir ? { barDir: mapped.barDir } : {}),
    ...(spec.options.stacked && mapped.barDir ? { barGrouping: 'stacked' } : {}),
    chartColors: CHART_PALETTE.light.map((c) => c.replace('#', '')),
    showLegend: spec.series.length > 1 || spec.type === 'pie' || spec.type === 'donut',
    legendPos: 'b',
    showTitle: !!spec.title,
    title: spec.title || undefined,
    titleFontSize: 13,
    showValue: spec.options.valueLabels === 'all',
    catAxisLabelFontSize: 9,
    valAxisLabelFontSize: 9,
    dataLabelFontSize: 9,
    holeSize: spec.type === 'donut' ? 58 : undefined,
  });
}

/** Convert a block's markdown into pptxgenjs text runs. */
function pptxRuns(md, style) {
  const blocks = parseMarkdown(md);
  const runs = [];
  const baseSize = PT(style.fontSize ?? 20);

  blocks.forEach((block, bi) => {
    const breakBefore = bi > 0;
    switch (block.type) {
      case 'heading': {
        const size = Math.max(baseSize, PT(HEADING_SIZE[block.level - 1] ?? 20));
        pushRuns(runs, block.runs, { bold: true, fontSize: size, breakLine: true }, breakBefore);
        break;
      }
      case 'list':
        block.items.forEach((item, i) => {
          pushRuns(
            runs,
            item.runs,
            {
              bullet: block.ordered ? { type: 'number' } : true,
              indentLevel: item.level,
              breakLine: true,
              fontSize: baseSize,
            },
            breakBefore && i === 0
          );
        });
        break;
      case 'quote':
        pushRuns(runs, block.runs, { italic: true, breakLine: true, fontSize: baseSize }, breakBefore);
        break;
      case 'code':
        runs.push({
          text: block.text,
          options: { fontFace: 'Consolas', fontSize: baseSize * 0.85, breakLine: true },
        });
        break;
      case 'table':
        block.rows.forEach((row) => {
          runs.push({ text: row.map(runsToText).join('  |  '), options: { fontSize: baseSize * 0.9, breakLine: true } });
        });
        break;
      case 'image':
        if (block.alt) runs.push({ text: block.alt, options: { italic: true, fontSize: baseSize, breakLine: true } });
        break;
      case 'hr':
        runs.push({ text: '─'.repeat(24), options: { color: 'D1D5DB', breakLine: true } });
        break;
      default:
        pushRuns(runs, block.runs, { breakLine: true, fontSize: baseSize }, breakBefore);
    }
  });

  return runs;
}

function pushRuns(out, runs, options, breakBefore) {
  if (breakBefore && out.length) out.push({ text: '', options: { breakLine: true } });
  runs.forEach((run, i) => {
    out.push({
      text: run.text,
      options: {
        ...options,
        bold: options.bold || run.bold || undefined,
        italic: options.italic || run.italic || undefined,
        strike: run.strike || undefined,
        fontFace: run.code ? 'Consolas' : undefined,
        hyperlink: run.link ? { url: run.link } : undefined,
        // Only the last run of a logical line carries the break.
        breakLine: i === runs.length - 1 ? options.breakLine : false,
      },
    });
  });
}

/* ------------------------------------------------------------------- docx */

/**
 * Export a document to .docx.
 *
 * Page setup comes from the section's own `page` block, and the per-paragraph
 * overrides in `meta.json` map onto Word's alignment, indent and spacing — the
 * same information, expressed in Word's vocabulary.
 */
export async function exportDocDocx(project) {
  const docx = await import('docx');
  const {
    Document, Packer, Paragraph, TextRun, HeadingLevel, AlignmentType,
    Table, TableRow, TableCell, WidthType, BorderStyle, ExternalHyperlink, ImageRun,
  } = docx;

  const sections = [];

  for (const section of project.sections) {
    const page = section.page ?? {};
    const size = PAGE_SIZES[page.size] ?? PAGE_SIZES.A4;
    const margin = page.margin ?? { top: 72, right: 72, bottom: 72, left: 72 };
    const children = [];

    for (const block of section.blocks) {
      const override = block.override ?? {};
      const parsed = parseMarkdown(block.md);

      // A .docx has no chart primitive we can write, so a chart becomes the table
      // of its own numbers with a caption naming the shape. Nothing is lost that a
      // reader needs, and Word can turn the table into a chart in two clicks.
      const chartSpec = parseChartBlock(block.md);
      if (chartSpec) {
        children.push(
          new Paragraph({
            children: [new TextRun({ text: chartSpec.title || '차트', bold: true, size: 26 })],
            spacing: { before: 200, after: 80 },
          })
        );
        const rows = chartToMarkdownTable(chartSpec)
          .split('\n')
          .filter((line) => line.startsWith('|') && !/^\|[\s:|-]+\|$/.test(line))
          .map((line) => line.replace(/^\|/, '').replace(/\|$/, '').split('|').map((c) => c.trim()));
        if (rows.length) {
          children.push(
            new Table({
              width: { size: 100, type: WidthType.PERCENTAGE },
              rows: rows.map(
                (row, r) =>
                  new TableRow({
                    tableHeader: r === 0,
                    children: row.map(
                      (cell) =>
                        new TableCell({
                          children: [new Paragraph({ children: [new TextRun({ text: cell, bold: r === 0 })] })],
                          shading: r === 0 ? { fill: 'F1F5F9' } : undefined,
                        })
                    ),
                  })
              ),
            })
          );
        }
        children.push(
          new Paragraph({
            children: [
              new TextRun({
                text: `표 데이터 · ${describeChart(chartSpec)}`,
                italics: true,
                size: 18,
                color: '6B7280',
              }),
            ],
            spacing: { after: 200 },
          })
        );
        continue;
      }

      for (const node of parsed) {
        switch (node.type) {
          case 'heading':
            children.push(
              new Paragraph({
                heading: [HeadingLevel.HEADING_1, HeadingLevel.HEADING_2, HeadingLevel.HEADING_3,
                  HeadingLevel.HEADING_4, HeadingLevel.HEADING_5, HeadingLevel.HEADING_6][node.level - 1],
                children: docxRuns(node.runs, override, docx),
                ...paragraphOptions(override, docx),
              })
            );
            break;
          case 'list':
            node.items.forEach((item, i) => {
              children.push(
                new Paragraph({
                  children: docxRuns(item.runs, override, docx),
                  ...(node.ordered
                    ? { numbering: { reference: 'ai-ordered', level: item.level } }
                    : { bullet: { level: item.level } }),
                  ...paragraphOptions(override, docx),
                })
              );
            });
            break;
          case 'quote':
            children.push(
              new Paragraph({
                children: docxRuns(node.runs, { ...override, style: { italic: true, ...(override.style ?? {}) } }, docx),
                indent: { left: TWIP(36 + (override.indent ?? 0)) },
                border: { left: { style: BorderStyle.SINGLE, size: 12, color: 'D1D5DB', space: 8 } },
                ...(override.align ? { alignment: alignment(override.align, AlignmentType) } : {}),
              })
            );
            break;
          case 'code':
            for (const line of node.text.split('\n')) {
              children.push(
                new Paragraph({
                  children: [new TextRun({ text: line || ' ', font: 'Consolas', size: 20 })],
                  shading: { fill: 'F3F4F6' },
                  spacing: { before: 0, after: 0 },
                })
              );
            }
            break;
          case 'table': {
            const rows = node.rows.map(
              (row, r) =>
                new TableRow({
                  tableHeader: r === 0,
                  children: row.map(
                    (cell) =>
                      new TableCell({
                        children: [new Paragraph({ children: docxRuns(cell, r === 0 ? { style: { bold: true } } : {}, docx) })],
                        shading: r === 0 ? { fill: 'F1F5F9' } : undefined,
                      })
                  ),
                })
            );
            children.push(new Table({ rows, width: { size: 100, type: WidthType.PERCENTAGE } }));
            children.push(new Paragraph({ text: '' }));
            break;
          }
          case 'image': {
            const file = await resolveAsset(project.dir, node.src);
            if (file) {
              try {
                const data = await fs.readFile(file);
                children.push(
                  new Paragraph({
                    children: [new ImageRun({ data, transformation: { width: 480, height: 300 }, type: imageType(file) })],
                    alignment: AlignmentType.CENTER,
                  })
                );
                if (node.alt) {
                  children.push(
                    new Paragraph({
                      children: [new TextRun({ text: node.alt, italics: true, size: 18, color: '6B7280' })],
                      alignment: AlignmentType.CENTER,
                    })
                  );
                }
                break;
              } catch {
                /* fall through to the alt-text placeholder */
              }
            }
            children.push(
              new Paragraph({
                children: [new TextRun({ text: node.alt || '(이미지)', italics: true, color: '9CA3AF' })],
                alignment: AlignmentType.CENTER,
              })
            );
            break;
          }
          case 'hr':
            children.push(
              new Paragraph({
                text: '',
                border: { bottom: { style: BorderStyle.SINGLE, size: 6, color: 'D1D5DB' } },
              })
            );
            break;
          default:
            children.push(
              new Paragraph({
                children: docxRuns(node.runs, override, docx),
                ...paragraphOptions(override, docx),
              })
            );
        }
      }
    }

    sections.push({
      properties: {
        page: {
          size: { width: TWIP(size.w * 0.75), height: TWIP(size.h * 0.75) },
          margin: {
            top: TWIP(PT(margin.top ?? 72)),
            right: TWIP(PT(margin.right ?? 72)),
            bottom: TWIP(PT(margin.bottom ?? 72)),
            left: TWIP(PT(margin.left ?? 72)),
          },
        },
      },
      children: children.length ? children : [new Paragraph({ text: '' })],
    });
  }

  const doc = new Document({
    title: project.manifest?.title ?? 'AI Studio Doc',
    numbering: {
      config: [
        {
          reference: 'ai-ordered',
          levels: [0, 1, 2].map((level) => ({
            level,
            format: 'decimal',
            text: `%${level + 1}.`,
            alignment: docx.AlignmentType.START,
            style: { paragraph: { indent: { left: TWIP(36 * (level + 1)), hanging: TWIP(18) } } },
          })),
        },
      ],
    },
    sections,
  });

  return Packer.toBuffer(doc);
}

function docxRuns(runs, override, docx) {
  const { TextRun, ExternalHyperlink } = docx;
  const style = override?.style ?? {};
  return (runs ?? []).map((run) => {
    const options = {
      text: run.text,
      bold: run.bold || style.bold || undefined,
      italics: run.italic || style.italic || undefined,
      strike: run.strike || undefined,
      underline: style.underline ? {} : undefined,
      font: run.code ? 'Consolas' : undefined,
      color: style.color ? hex(style.color) : undefined,
      size: style.fontSize ? Math.round(PT(style.fontSize) * 2) : undefined,
    };
    const textRun = new TextRun(options);
    return run.link ? new ExternalHyperlink({ children: [textRun], link: run.link }) : textRun;
  });
}

function paragraphOptions(override, docx) {
  const { AlignmentType } = docx;
  const out = {};
  if (override?.align) out.alignment = alignment(override.align, AlignmentType);
  if (override?.indent) out.indent = { left: TWIP(override.indent) };
  if (override?.spacing) {
    out.spacing = {
      before: override.spacing.before ? TWIP(PT(override.spacing.before)) : undefined,
      after: override.spacing.after ? TWIP(PT(override.spacing.after)) : undefined,
    };
  }
  return out;
}

function alignment(align, AlignmentType) {
  return {
    left: AlignmentType.LEFT,
    center: AlignmentType.CENTER,
    right: AlignmentType.RIGHT,
    justify: AlignmentType.JUSTIFIED,
  }[align] ?? AlignmentType.LEFT;
}

function imageType(file) {
  const ext = path.extname(file).toLowerCase().replace('.', '');
  return ext === 'jpeg' ? 'jpg' : ext === 'svg' ? 'svg' : ext;
}

/* ------------------------------------------------------------------- xlsx */

/**
 * Export a workbook to .xlsx.
 *
 * Formulas go across as formulas (not their cached values), so the file stays
 * live in Excel. Number formats, styles, merges, column widths, frozen panes and
 * named ranges all have direct equivalents.
 */
export async function exportGridXlsx(project) {
  const ExcelJS = (await import('exceljs')).default;
  const wb = new ExcelJS.Workbook();
  wb.creator = 'AI Studio';
  wb.created = new Date(project.manifest?.created ?? Date.now());
  wb.modified = new Date();

  for (const sheet of project.sheets) {
    const ws = wb.addWorksheet(sheet.name || '시트', {
      views: [
        {
          state: 'frozen',
          xSplit: sheet.frozen?.cols ?? 0,
          ySplit: sheet.frozen?.rows ?? 0,
        },
      ],
    });

    const { cells } = recalcSheet({ cells: sheet.cells, names: sheet.names });
    const range = usedRange(cells);

    for (const [ref, cell] of Object.entries(cells)) {
      const target = ws.getCell(ref);
      if (cell.f) {
        target.value = { formula: cell.f.replace(/^=/, ''), result: excelValue(cell) };
      } else {
        target.value = excelValue(cell);
      }
      if (cell.fmt) target.numFmt = excelNumFmt(cell.fmt);

      const style = cell.style ?? {};
      if (style.bold || style.italic || style.underline || style.color) {
        target.font = {
          bold: !!style.bold,
          italic: !!style.italic,
          underline: !!style.underline,
          ...(style.color ? { color: { argb: `FF${hex(style.color)}` } } : {}),
        };
      }
      if (style.bg) {
        target.fill = { type: 'pattern', pattern: 'solid', fgColor: { argb: `FF${hex(style.bg)}` } };
      }
      if (style.align) target.alignment = { horizontal: style.align };
      if (style.border) {
        const edge = { style: 'thin', color: { argb: 'FF9CA3AF' } };
        target.border = {
          ...(style.border.t ? { top: edge } : {}),
          ...(style.border.b ? { bottom: edge } : {}),
          ...(style.border.l ? { left: edge } : {}),
          ...(style.border.r ? { right: edge } : {}),
        };
      }
    }

    // exceljs cannot write chart parts, so each chart is recorded as a labelled
    // block of its own numbers below the data — enough for the recipient to insert
    // a native Excel chart from it.
    if (sheet.charts?.length) {
      const range = usedRange(cells);
      let row = (range ? range.maxRow + 1 : 0) + 3;
      for (const chart of sheet.charts) {
        const resolved = resolveChartSpec(chart.spec, { ...sheet, cells });
        ws.getCell(row, 1).value = `차트: ${resolved.title || CHART_TYPE_LABELS[resolved.type]}`;
        ws.getCell(row, 1).font = { bold: true };
        row++;
        ws.getCell(row, 1).value = describeChart(resolved);
        ws.getCell(row, 1).font = { italic: true, size: 9, color: { argb: 'FF6B7280' } };
        row += 1;

        const header = ['구분', ...resolved.series.map((sr, i) => sr.name || `계열 ${i + 1}`)];
        header.forEach((text, c) => {
          const cell = ws.getCell(row, c + 1);
          cell.value = text;
          cell.font = { bold: true };
          cell.fill = { type: 'pattern', pattern: 'solid', fgColor: { argb: 'FFF1F5F9' } };
        });
        row++;

        const points = resolved.labels.length || Math.max(...resolved.series.map((sr) => sr.values.length), 0);
        for (let i = 0; i < points; i++) {
          ws.getCell(row, 1).value = resolved.labels[i] ?? i + 1;
          resolved.series.forEach((sr, c) => {
            const v = sr.values[i];
            if (v !== null && v !== undefined) ws.getCell(row, c + 2).value = v;
          });
          row++;
        }
        row += 2;
      }
    }

    for (const spec of sheet.merges ?? []) {
      try {
        ws.mergeCells(spec);
      } catch {
        /* an invalid or overlapping merge is skipped rather than failing the export */
      }
    }

    const cols = range ? range.maxCol + 1 : 0;
    for (let c = 0; c < cols; c++) {
      const width = sheet.colWidths?.[indexToCol(c)];
      // Excel column width is measured in characters, roughly px / 7.
      ws.getColumn(c + 1).width = width ? Math.round(width / 7) : 13;
    }
    for (const [row, height] of Object.entries(sheet.rowHeights ?? {})) {
      ws.getRow(Number(row)).height = Math.round(PT(height));
    }

    for (const [name, target] of Object.entries(sheet.names ?? {})) {
      const safe = name.replace(/[^\w가-힣_.]/g, '_');
      try {
        wb.definedNames.add(`'${ws.name}'!${target}`, safe);
      } catch {
        /* Excel rejects some names; skip rather than fail */
      }
    }
  }

  return wb.xlsx.writeBuffer().then((data) => Buffer.from(data));
}

function excelValue(cell) {
  if (cell.t === 'e') return { error: String(cell.v ?? '#VALUE!') };
  if (cell.v === null || cell.v === undefined) return null;
  if (cell.t === 'd' && typeof cell.v === 'number') return fromSerial(cell.v);
  return cell.v;
}

/** Our format strings are already Excel-compatible; date codes need lowering. */
function excelNumFmt(fmt) {
  return /[ymd]/i.test(fmt) && !/[#0]/.test(fmt) ? fmt.toLowerCase() : fmt;
}

/* -------------------------------------------------------------------- csv */

/** CSV of a single sheet's used range, values as displayed. */
export function exportGridCsv(project, sheetIndex = 0) {
  const sheet = project.sheets[sheetIndex];
  if (!sheet) return '';
  const { cells } = recalcSheet({ cells: sheet.cells, names: sheet.names });
  const range = usedRange(cells);
  if (!range) return '';

  const lines = [];
  for (let r = 0; r <= range.maxRow; r++) {
    const row = [];
    for (let c = 0; c <= range.maxCol; c++) row.push(csvCell(displayValue(cells[toRef(c, r)])));
    lines.push(row.join(','));
  }
  // Excel needs a BOM to read UTF-8 CSV correctly on Windows.
  return `﻿${lines.join('\r\n')}\r\n`;
}

function csvCell(text) {
  const value = String(text ?? '');
  return /[",\r\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value;
}

/* ----------------------------------------------------------------- helpers */

function hex(color) {
  const value = String(color ?? '').replace('#', '').trim();
  if (/^[0-9a-fA-F]{6}$/.test(value)) return value.toUpperCase();
  if (/^[0-9a-fA-F]{3}$/.test(value)) {
    return value.split('').map((ch) => ch + ch).join('').toUpperCase();
  }
  return '000000';
}

function imageSource(md) {
  const m = String(md ?? '').match(/!\[[^\]]*\]\(([^)\s]+)/);
  return m ? m[1] : null;
}

function imageAlt(md) {
  const m = String(md ?? '').match(/!\[([^\]]*)\]/);
  return m ? m[1] : '';
}

function tableRows(md) {
  return String(md ?? '')
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.startsWith('|') && !/^\|[\s:|-]+\|?$/.test(line))
    .map((line) =>
      line.replace(/^\|/, '').replace(/\|$/, '').split('|').map((cell) => cell.trim().replace(/\*\*/g, ''))
    );
}

/**
 * Resolve an image reference to a real file inside the project.
 * Anything that escapes the project folder, or is a remote URL, returns null —
 * the export must not read outside the document or fetch over the network.
 */
async function resolveAsset(projectDir, src) {
  if (!src || /^(https?:|data:)/i.test(src)) return null;
  const abs = path.resolve(projectDir, src.replace(/^\.\//, ''));
  const rel = path.relative(path.resolve(projectDir), abs);
  if (rel.startsWith('..') || path.isAbsolute(rel)) return null;
  try {
    const stat = await fs.stat(abs);
    return stat.isFile() ? abs : null;
  } catch {
    return null;
  }
}

/* ---------------------------------------------------------------- registry */

export const EXPORTERS = {
  pptx: { type: 'deck', mime: 'application/vnd.openxmlformats-officedocument.presentationml.presentation', run: exportDeckPptx },
  docx: { type: 'doc', mime: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', run: exportDocDocx },
  xlsx: { type: 'grid', mime: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', run: exportGridXlsx },
  csv: { type: 'grid', mime: 'text/csv; charset=utf-8', run: async (p) => Buffer.from(exportGridCsv(p), 'utf8') },
};
