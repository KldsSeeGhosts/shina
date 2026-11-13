//! KSG `VirtualTablePreview.tsx`: the delimited table's measured column widths
//! and the columns a scroll offset puts in view. The view side lives in
//! `attachment_preview.rs`.
use super::attachment_text_preview::Document;
use rune::TextStyle;
use std::ops::Range;

/// KSG `rowHeader`: the row-number gutter's width in points.
pub(crate) const GUTTER: f32 = 56.0;
/// KSG `min-h-[29px]`: the header and every body row's height.
pub(crate) const ROW_HEIGHT: f32 = 29.0;
/// KSG `min-w-32`/`max-w-80`: each column's width bounds.
pub(crate) const MIN_COLUMN: f32 = 128.0;
pub(crate) const MAX_COLUMN: f32 = 320.0;
/// KSG `px-3`: the horizontal padding inside a cell, both sides together.
const CELL_PAD_X: f32 = 24.0;
/// KSG measures real cell widths only for tables of at most 30 columns.
const MEASURED_MAX_COLUMNS: usize = 30;
/// KSG's hidden measure table holds the header and `getRows(0).slice(1, 20)`.
const MEASURED_SAMPLE_END: usize = 20;
/// Distribution passes while columns grow toward the table's width.
const DISTRIBUTE_PASSES: usize = 8;

pub(crate) struct Table {
    /// The first row's cells (KSG `header`). A short row's missing cells read
    /// `column_label` instead.
    pub(crate) header: Vec<String>,
    /// KSG `meta.columns`: the widest row's cell count.
    pub(crate) columns: usize,
    /// KSG `sizes`: every column's resolved width.
    pub(crate) sizes: Vec<f32>,
    /// KSG `offsets`: `columns + 1` cumulative left edges, `offsets[c]` to
    /// `offsets[c + 1]` is column `c`.
    pub(crate) offsets: Vec<f32>,
}

impl Table {
    /// KSG `sizes` and `offsets`. At most `MEASURED_MAX_COLUMNS` columns get a
    /// hidden-table pass: each column's content bounds come from the header and
    /// rows 1..20 in the two styles the view draws with, then the browser's
    /// automatic layout is approximated as a lerp between min- and max-content
    /// inside the table width `max(viewport - GUTTER, columns * MIN_COLUMN)`.
    /// Wider tables take KSG's `columnWidth` formula for every column.
    /// `viewport` is the scroll viewport's width; 0 falls back to KSG's
    /// pre-measure defaults.
    pub(crate) fn measure(
        document: &Document,
        viewport: f32,
        cell: TextStyle,
        header: TextStyle,
    ) -> Table {
        let columns = document.columns;
        let header_cells = if document.len() > 0 {
            document.row(0, true).unwrap_or_default()
        } else {
            Vec::new()
        };
        let sizes = if columns <= MEASURED_MAX_COLUMNS && document.len() > 0 {
            measured_sizes(document, &header_cells, viewport, cell, header)
        } else {
            vec![fallback(viewport, columns); columns]
        };
        let mut offsets = Vec::with_capacity(columns + 1);
        offsets.push(0.0);
        for size in &sizes {
            offsets.push(*offsets.last().unwrap() + size);
        }
        Table {
            header: header_cells,
            columns,
            sizes,
            offsets,
        }
    }

    /// KSG `firstColumn`/`lastColumn`: the columns intersecting the horizontal
    /// window `[left - GUTTER, left + viewport - GUTTER]`, one column of
    /// overscan on each side.
    pub(crate) fn visible(&self, left: f32, viewport: f32) -> Range<usize> {
        let first = self
            .sizes
            .iter()
            .enumerate()
            .find(|(column, _)| self.offsets[column + 1] > left - GUTTER)
            .map_or(0, |(column, _)| column)
            .saturating_sub(1);
        let last = self
            .offsets
            .iter()
            .position(|offset| *offset >= left + viewport - GUTTER)
            .map_or(self.columns, |index| (index + 1).min(self.columns));
        first..last.max(first)
    }

    /// KSG `offset(props.columns)`: the cells' total width, without the gutter.
    pub(crate) fn width(&self) -> f32 {
        self.offsets[self.columns]
    }
}

/// KSG's `columnWidth`, used when no measurement ran or the table is too wide
/// to measure: `clamp(128, 320, (viewport - GUTTER) / min(columns, 6))`.
fn fallback(viewport: f32, columns: usize) -> f32 {
    ((viewport - GUTTER) / columns.min(6).max(1) as f32).clamp(MIN_COLUMN, MAX_COLUMN)
}

/// The hidden measure table's per-column bounds: `lo` is the widest unbreakable
/// run plus padding (`min-w-32`'s floor, `max-w-80`'s cap), `hi` the one-line
/// width plus padding. Header cells measure in the header style, body cells in
/// the cell style, as KSG's `font-medium` `th` vs `tabular-nums` `td` do.
fn measured_sizes(
    document: &Document,
    header: &[String],
    viewport: f32,
    cell: TextStyle,
    header_style: TextStyle,
) -> Vec<f32> {
    let columns = document.columns;
    let mut lo = vec![0.0f32; columns];
    let mut hi = vec![0.0f32; columns];
    let mut grow = |column: usize, text: &str, style: TextStyle| {
        let (min, max) = rune::text::content_widths(text, &[], style);
        lo[column] = lo[column].max(min);
        hi[column] = hi[column].max(max);
    };
    for (column, text) in header.iter().enumerate().take(columns) {
        grow(column, text, header_style);
    }
    for row in 1..MEASURED_SAMPLE_END.min(document.len()) {
        let Ok(cells) = document.row(row, true) else {
            continue;
        };
        for (column, text) in cells.iter().enumerate().take(columns) {
            grow(column, text, cell);
        }
    }
    for column in 0..columns {
        lo[column] = (lo[column] + CELL_PAD_X).clamp(MIN_COLUMN, MAX_COLUMN);
        hi[column] = (hi[column] + CELL_PAD_X).clamp(lo[column], MAX_COLUMN);
    }
    // The hidden table is `max(viewport - GUTTER, columns * MIN_COLUMN)` wide:
    // columns between their bounds share the table width, capped at `MAX_COLUMN`.
    let target = (viewport - GUTTER).max(columns as f32 * MIN_COLUMN);
    let sum_lo: f32 = lo.iter().sum();
    let sum_hi: f32 = hi.iter().sum();
    if sum_lo >= target {
        return lo;
    }
    if sum_hi <= target {
        let mut sizes = hi;
        for _ in 0..DISTRIBUTE_PASSES {
            let spare = target - sizes.iter().sum::<f32>();
            if spare < 0.5 {
                break;
            }
            let weight: f32 = sizes.iter().filter(|w| **w < MAX_COLUMN).sum();
            if weight <= 0.0 {
                break;
            }
            for size in &mut sizes {
                if *size < MAX_COLUMN {
                    *size = (*size + spare * *size / weight).min(MAX_COLUMN);
                }
            }
        }
        return sizes;
    }
    let share = (target - sum_lo) / (sum_hi - sum_lo);
    lo.iter()
        .zip(&hi)
        .map(|(lo, hi)| lo + (hi - lo) * share)
        .collect()
}

/// KSG `columnLabel`: A..Z, AA..AZ, BA.. — the text a header cell past the
/// first row's length falls back to.
pub(crate) fn column_label(index: usize) -> String {
    let mut label = String::new();
    let mut value = index + 1;
    while value > 0 {
        label.insert(0, char::from(b'A' + ((value - 1) % 26) as u8));
        value = (value - 1) / 26;
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::attachment_text_preview::{MAX_BYTES, MAX_COLUMNS, MAX_RECORD, MAX_ROWS};

    const CELL: TextStyle = TextStyle::new(12.0, 400.0).family(rune::Family::SystemMono);
    const HEAD: TextStyle = TextStyle::new(12.0, 600.0);

    fn index(bytes: &[u8], delimiter: Option<u8>) -> Result<Document, String> {
        Document::index(bytes.to_vec(), delimiter, true)
    }

    #[test]
    fn delimited_rows_parse_with_ksg_rules() {
        let doc = index(
            b"\xef\xbb\xbfname,value\r\n\"multi\nline\",\"a\"\"b\"\r\nlast,2",
            Some(b','),
        )
        .unwrap();
        assert_eq!(doc.len(), 3);
        assert_eq!(doc.columns, 2);
        assert_eq!(doc.row(0, true).unwrap(), ["name", "value"]);
        assert_eq!(doc.row(1, true).unwrap(), ["multi\nline", "a\"b"]);
        assert_eq!(doc.row(2, true).unwrap(), ["last", "2"]);

        let tsv = index(b"a\tb\tc\n1\t2,3\t\"x\ty\"", Some(b'\t')).unwrap();
        assert_eq!(tsv.row(0, true).unwrap(), ["a", "b", "c"]);
        assert_eq!(tsv.row(1, true).unwrap(), ["1", "2,3", "x\ty"]);

        // Ragged rows keep their own cell counts; the table's column count is
        // the widest row's.
        let ragged = index(b"a,b,c\n1,2\nx,y,z,w\n", Some(b',')).unwrap();
        assert_eq!(ragged.len(), 3);
        assert_eq!(ragged.columns, 4);
        assert_eq!(ragged.row(1, true).unwrap(), ["1", "2"]);
        assert_eq!(ragged.row(2, true).unwrap().len(), 4);

        // A trailing newline does not add an empty row, and a bare CR splits.
        assert_eq!(index(b"a,b\n", Some(b',')).unwrap().len(), 1);
        assert_eq!(index(b"a\rb\rc", Some(b',')).unwrap().len(), 3);
    }

    #[test]
    fn preview_limits_report_ksg_messages() {
        assert_eq!(
            index(&vec![0u8; MAX_BYTES + 1], None).err().unwrap(),
            "Text previews support files up to 256 MB."
        );
        assert_eq!(
            index(&vec![b'\n'; MAX_ROWS + 1], None).err().unwrap(),
            "Text previews support up to 5 million rows."
        );
        assert_eq!(
            index(&vec![b'a'; MAX_RECORD + 2], None).err().unwrap(),
            "A line or record exceeds the 1 MB preview limit."
        );
        assert_eq!(
            index(b"binary\0", None).err().unwrap(),
            "This file contains binary data."
        );
        assert_eq!(
            index(b"\xff", None).unwrap().row(0, true).unwrap_err(),
            "This file is not UTF-8 text. Open it in another app to view its contents."
        );
        // Columns past the 1,024 cap truncate silently; the notice is the view's.
        let mut row = Vec::new();
        for _ in 0..MAX_COLUMNS + 3 {
            row.extend_from_slice(b"x,");
        }
        row.pop();
        let doc = index(&row, Some(b',')).unwrap();
        assert_eq!(doc.columns, MAX_COLUMNS);
        assert_eq!(doc.row(0, true).unwrap().len(), MAX_COLUMNS);
    }

    #[test]
    fn column_labels_count_like_ksg() {
        let labels: Vec<String> = (0..30).map(column_label).collect();
        assert_eq!(
            labels,
            [
                "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P",
                "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "AA", "AB", "AC", "AD"
            ]
        );
    }

    #[test]
    fn measured_widths_stay_inside_the_ksg_clamps() {
        let doc = index(b"a,b\n1,2", Some(b',')).unwrap();
        let table = Table::measure(&doc, 256.0, CELL, HEAD);
        assert_eq!(table.sizes, vec![MIN_COLUMN, MIN_COLUMN]);

        // A wide viewport hands spare width out until every column hits the
        // `max-w-80` cap; a narrow one keeps columns at their floors.
        let wide = format!("short,{}", "cell ".repeat(200));
        let doc = index(wide.as_bytes(), Some(b',')).unwrap();
        let table = Table::measure(&doc, 4096.0, CELL, HEAD);
        assert_eq!(table.sizes, vec![MAX_COLUMN, MAX_COLUMN]);
        let table = Table::measure(&doc, 256.0, CELL, HEAD);
        assert!(
            table
                .sizes
                .iter()
                .all(|w| (MIN_COLUMN..=MAX_COLUMN).contains(w))
        );
    }

    #[test]
    fn visible_columns_cover_the_viewport_at_the_edges_and_middle() {
        // Past 30 columns every column takes `columnWidth`: (800 - 56) / 6 = 124,
        // clamped to 128.
        let row = (0..200).map(|c| format!("c{c},")).collect::<String>();
        let doc = index(row.trim_end_matches(',').as_bytes(), Some(b',')).unwrap();
        let table = Table::measure(&doc, 800.0, CELL, HEAD);
        assert_eq!(table.columns, 200);
        assert!(table.sizes.iter().all(|w| *w == MIN_COLUMN));
        assert_eq!(table.width(), 200.0 * MIN_COLUMN);

        // Offset 0: the first column whose right edge clears -56 is column 0,
        // minus one overscan; the last is the offset at or past 744, +1.
        assert_eq!(table.visible(0.0, 800.0), 0..7);
        // Mid-scroll: offsets[24] = 3072 is the first right edge past 2944.
        assert_eq!(table.visible(3000.0, 800.0), 22..31);
        // The far right end: a 25,656-point table in an 800-point viewport
        // scrolls to 24,856; columns 192..200 intersect 24,800..25,600.
        assert_eq!(table.visible(24856.0, 800.0), 192..200);
        // A viewport never reported (width 0) still resolves a sane window.
        assert_eq!(table.visible(0.0, 0.0), 0..1);
    }
}
