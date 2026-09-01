#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::any::Any;
use std::sync::{Arc, Once};

use floem::context::{PaintCx, UpdateCx};
use floem::peniko::kurbo::{Point, Rect, Stroke};
use floem::reactive::{RwSignal, SignalGet, SignalUpdate, create_effect};
use floem::text::FONT_SYSTEM;
use floem::text::{Attrs, AttrsList, FamilyOwned, Style, TextLayout, Weight};
use floem::{View, ViewId};
use floem_renderer::Renderer;
use matcha_terminal::{
    CellPoint, CellRange, CursorShape, SearchResult, TerminalColor, TerminalFrame, UnderlineStyle,
};

const PADDING_X: f64 = 8.0;
const PADDING_Y: f64 = 6.0;
static REGISTER_BUNDLED_FONTS: Once = Once::new();

#[derive(Clone)]
struct TerminalPaintUpdate {
    frame: Arc<TerminalFrame>,
    font_size: f32,
    font_family: String,
    line_height: f32,
    search: SearchResult,
    preedit: String,
    cursor_visible: bool,
    background: TerminalColor,
}

pub struct TerminalView {
    id: ViewId,
    state: TerminalPaintUpdate,
    rows: Vec<Vec<CachedGlyph>>,
    display_scale: RwSignal<f64>,
}

#[derive(Clone, Copy)]
pub struct TerminalViewSignals {
    pub frame: RwSignal<Arc<TerminalFrame>>,
    pub font_size: RwSignal<f32>,
    pub font_family: RwSignal<String>,
    pub line_height: RwSignal<f32>,
    pub search: RwSignal<SearchResult>,
    pub preedit: RwSignal<String>,
    pub cursor_visible: RwSignal<bool>,
    pub background: RwSignal<TerminalColor>,
    pub display_scale: RwSignal<f64>,
}

struct CachedGlyph {
    column: usize,
    columns: usize,
    layout: TextLayout,
    y_offset: f64,
    underline: UnderlineStyle,
    underline_color: TerminalColor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BackgroundRun {
    row: usize,
    start_column: usize,
    end_column: usize,
    color: TerminalColor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontResolution {
    pub family: String,
    pub used_fallback: bool,
}

pub fn register_bundled_fonts() {
    const FONTS: [&[u8]; 4] = [
        include_bytes!("../assets/fonts/JetBrainsMonoNL-Regular.ttf"),
        include_bytes!("../assets/fonts/JetBrainsMonoNL-Bold.ttf"),
        include_bytes!("../assets/fonts/JetBrainsMonoNL-Italic.ttf"),
        include_bytes!("../assets/fonts/JetBrainsMonoNL-BoldItalic.ttf"),
    ];
    REGISTER_BUNDLED_FONTS.call_once(|| {
        let mut font_system = FONT_SYSTEM.lock();
        for font in FONTS {
            font_system.db_mut().load_font_data(font.to_vec());
        }
    });
}

#[must_use]
pub fn resolve_font_family(requested: &str) -> FontResolution {
    let requested = requested.trim();
    let resolved = (!requested.is_empty()).then(|| {
        let font_system = FONT_SYSTEM.lock();
        font_system.db().faces().find_map(|face| {
            face.families.iter().find_map(|(family, _)| {
                family
                    .eq_ignore_ascii_case(requested)
                    .then(|| family.clone())
            })
        })
    });
    match resolved.flatten() {
        Some(family) => FontResolution {
            family,
            used_fallback: false,
        },
        None => FontResolution {
            family: matcha_config::DEFAULT_FONT_FAMILY.into(),
            used_fallback: true,
        },
    }
}

pub fn terminal_view(signals: TerminalViewSignals) -> TerminalView {
    let id = ViewId::new();
    let initial = TerminalPaintUpdate {
        frame: signals.frame.get_untracked(),
        font_size: signals.font_size.get_untracked(),
        font_family: signals.font_family.get_untracked(),
        line_height: signals.line_height.get_untracked(),
        search: signals.search.get_untracked(),
        preedit: signals.preedit.get_untracked(),
        cursor_visible: signals.cursor_visible.get_untracked(),
        background: signals.background.get_untracked(),
    };
    create_effect(move |_| {
        id.update_state(TerminalPaintUpdate {
            frame: signals.frame.get(),
            font_size: signals.font_size.get(),
            font_family: signals.font_family.get(),
            line_height: signals.line_height.get(),
            search: signals.search.get(),
            preedit: signals.preedit.get(),
            cursor_visible: signals.cursor_visible.get(),
            background: signals.background.get(),
        });
    });
    let mut view = TerminalView {
        id,
        state: initial,
        rows: Vec::new(),
        display_scale: signals.display_scale,
    };
    view.rebuild_all_rows();
    view
}

impl TerminalView {
    fn rebuild_all_rows(&mut self) {
        self.rows = (0..self.state.frame.size.lines)
            .map(|row| build_row(&self.state, row))
            .collect();
    }

    fn rebuild_damaged_rows(&mut self) {
        let damaged = match &self.state.frame.damage {
            matcha_terminal::FrameDamage::Full => {
                self.rebuild_all_rows();
                return;
            }
            matcha_terminal::FrameDamage::Partial(regions) => regions
                .iter()
                .map(|region| region.row)
                .collect::<std::collections::BTreeSet<_>>(),
            matcha_terminal::FrameDamage::None => return,
        };
        if self.rows.len() != self.state.frame.size.lines {
            self.rebuild_all_rows();
            return;
        }
        for row in damaged {
            if row < self.rows.len() {
                self.rows[row] = build_row(&self.state, row);
            }
        }
    }
}

impl View for TerminalView {
    fn id(&self) -> ViewId {
        self.id
    }

    fn update(&mut self, _cx: &mut UpdateCx, state: Box<dyn Any>) {
        if let Ok(state) = state.downcast::<TerminalPaintUpdate>() {
            let font_changed = self.state.font_size.to_bits() != state.font_size.to_bits()
                || self.state.font_family != state.font_family
                || self.state.line_height.to_bits() != state.line_height.to_bits();
            self.state = *state;
            if font_changed {
                self.rebuild_all_rows();
            } else {
                self.rebuild_damaged_rows();
            }
            self.id.request_paint();
        }
    }

    fn paint(&mut self, cx: &mut PaintCx) {
        let frame = &self.state.frame;
        let font_size = self.state.font_size;
        let (cell_width, line_height) =
            cell_metrics(font_size, &self.state.font_family, self.state.line_height);
        let background = color(self.state.background);
        let scale = cx.scale().max(1.0);
        if self.display_scale.get_untracked().to_bits() != scale.to_bits() {
            self.display_scale.set(scale);
        }
        let layout = self.id.get_layout().unwrap_or_default();
        cx.fill(
            &Rect::new(
                0.0,
                0.0,
                f64::from(layout.size.width),
                f64::from(layout.size.height),
            ),
            background,
            0.0,
        );

        for run in background_runs(frame) {
            let rect = cell_span_rect(
                run.row,
                run.start_column,
                run.end_column,
                cell_width,
                line_height,
                scale,
            );
            cx.fill(&rect, color(run.color), 0.0);
        }

        if let Some(selection) = frame.selection {
            for row in 0..frame.size.lines {
                if let Some((start, end)) = range_columns(selection, row, frame.size.columns) {
                    cx.fill(
                        &cell_span_rect(row, start, end, cell_width, line_height, scale),
                        floem::peniko::Color::rgba8(82, 112, 91, 190),
                        0.0,
                    );
                }
            }
        }

        for (index, range) in self.state.search.matches.iter().enumerate() {
            let search_color = if self.state.search.active == Some(index) {
                floem::peniko::Color::rgba8(215, 176, 74, 220)
            } else {
                floem::peniko::Color::rgba8(133, 112, 49, 170)
            };
            for row in range.start.row..=range.end.row {
                if let Some((start, end)) = range_columns(*range, row, frame.size.columns) {
                    cx.fill(
                        &cell_span_rect(row, start, end, cell_width, line_height, scale),
                        search_color,
                        0.0,
                    );
                }
            }
        }

        for (row, glyphs) in self.rows.iter().enumerate() {
            for glyph in glyphs {
                cx.draw_text(
                    &glyph.layout,
                    Point::new(
                        column_boundary(glyph.column, cell_width, scale),
                        snap_to_pixel(PADDING_Y + row as f64 * line_height + glyph.y_offset, scale),
                    ),
                );
                if glyph.underline == UnderlineStyle::None {
                    continue;
                }
                let y = snap_to_pixel(PADDING_Y + (row + 1) as f64 * line_height - 2.0, scale);
                cx.stroke(
                    &floem::peniko::kurbo::Line::new(
                        (column_boundary(glyph.column, cell_width, scale), y),
                        (
                            column_boundary(glyph.column + glyph.columns, cell_width, scale),
                            y,
                        ),
                    ),
                    color(glyph.underline_color),
                    &Stroke::new(1.0),
                );
            }
        }

        if self.state.cursor_visible {
            paint_cursor(cx, frame, cell_width, line_height, scale);
        }
        if !self.state.preedit.is_empty() {
            paint_preedit(
                cx,
                frame,
                &self.state.preedit,
                font_size,
                cell_width,
                line_height,
                scale,
            );
        }
    }
}

fn build_row(state: &TerminalPaintUpdate, row: usize) -> Vec<CachedGlyph> {
    let family = [FamilyOwned::Name(state.font_family.clone())];
    let cells = state
        .frame
        .cells
        .iter()
        .filter(|cell| cell.row == row && !cell.style.wide_spacer)
        .collect::<Vec<_>>();
    let mut glyphs = Vec::new();
    let mut index = 0;
    while index < cells.len() {
        let first = cells[index];
        let precise = first.style.wide || first.text.chars().count() != 1;
        let mut text = first.text.clone();
        let mut columns = if first.style.wide { 2 } else { 1 };
        let mut next = index + 1;
        if !precise {
            while let Some(cell) = cells.get(next) {
                let is_contiguous = cell.column == first.column + columns;
                let same_style = cell.foreground == first.foreground
                    && cell.style.bold == first.style.bold
                    && cell.style.italic == first.style.italic
                    && cell.style.underline == first.style.underline
                    && cell.underline_color == first.underline_color;
                let is_simple = !cell.style.wide && cell.text.chars().count() == 1;
                if !is_contiguous || !same_style || !is_simple {
                    break;
                }
                text.push_str(&cell.text);
                columns += 1;
                next += 1;
            }
        }
        if text.chars().any(|character| character != ' ') {
            let mut attrs = Attrs::new()
                .font_size(state.font_size)
                .family(&family)
                .color(color(first.foreground));
            if first.style.bold {
                attrs = attrs.weight(Weight::BOLD);
            }
            if first.style.italic {
                attrs = attrs.style(Style::Italic);
            }
            let mut layout = TextLayout::new();
            layout.set_text(&text, AttrsList::new(attrs));
            let layout_height = layout
                .layout_runs()
                .next()
                .map_or(f64::from(state.font_size), |run| f64::from(run.line_height));
            let cell_height = f64::from(state.font_size * state.line_height);
            glyphs.push(CachedGlyph {
                column: first.column,
                columns,
                layout,
                y_offset: ((cell_height - layout_height) / 2.0).max(0.0),
                underline: first.style.underline,
                underline_color: first.underline_color,
            });
        }
        index = next;
    }
    glyphs
}

pub fn cell_metrics(font_size: f32, font_family: &str, line_height: f32) -> (f64, f64) {
    let family = [FamilyOwned::Name(font_family.to_owned())];
    let mut layout = TextLayout::new();
    layout.set_text(
        "M",
        AttrsList::new(Attrs::new().font_size(font_size).family(&family)),
    );
    let width = layout
        .layout_runs()
        .next()
        .map_or(f64::from(font_size) * 0.62, |run| f64::from(run.line_w));
    (
        width.max(1.0),
        f64::from(
            font_size
                * line_height.clamp(
                    matcha_config::MIN_LINE_HEIGHT,
                    matcha_config::MAX_LINE_HEIGHT,
                ),
        ),
    )
}

pub fn point_to_cell(
    point: Point,
    font_size: f32,
    font_family: &str,
    line_height: f32,
    display_scale: f64,
    frame: &TerminalFrame,
) -> CellPoint {
    let (cell_width, line_height) = cell_metrics(font_size, font_family, line_height);
    let display_scale = display_scale.max(1.0);
    let column = find_cell_index(point.x, frame.size.columns, |index| {
        column_boundary(index, cell_width, display_scale)
    });
    let row = find_cell_index(point.y, frame.size.lines, |index| {
        row_boundary(index, line_height, display_scale)
    });
    CellPoint {
        row: row.min(frame.size.lines.saturating_sub(1)),
        column: column.min(frame.size.columns.saturating_sub(1)),
    }
}

fn paint_cursor(
    cx: &mut PaintCx,
    frame: &TerminalFrame,
    cell_width: f64,
    line_height: f64,
    scale: f64,
) {
    let cursor = frame.cursor;
    let rect = cell_span_rect(
        cursor.row,
        cursor.column,
        cursor.column + 1,
        cell_width,
        line_height,
        scale,
    );
    let cursor_color = floem::peniko::Color::rgba8(167, 213, 165, 210);
    match cursor.shape {
        CursorShape::Block => cx.stroke(&rect, cursor_color, &Stroke::new(1.5)),
        CursorShape::Underline => cx.fill(
            &Rect::new(rect.x0, rect.y1 - 2.0, rect.x1, rect.y1),
            cursor_color,
            0.0,
        ),
        CursorShape::Beam => cx.fill(
            &Rect::new(rect.x0, rect.y0, rect.x0 + 2.0, rect.y1),
            cursor_color,
            0.0,
        ),
        CursorShape::Hidden => {}
    }
}

fn paint_preedit(
    cx: &mut PaintCx,
    frame: &TerminalFrame,
    preedit: &str,
    font_size: f32,
    cell_width: f64,
    line_height: f64,
    scale: f64,
) {
    let mut text = TextLayout::new();
    text.set_text(
        preedit,
        AttrsList::new(
            Attrs::new()
                .font_size(font_size)
                .color(floem::peniko::Color::rgb8(232, 240, 232)),
        ),
    );
    cx.draw_text(
        &text,
        Point::new(
            column_boundary(frame.cursor.column, cell_width, scale),
            row_boundary(frame.cursor.row, line_height, scale),
        ),
    );
}

fn background_runs(frame: &TerminalFrame) -> Vec<BackgroundRun> {
    let mut runs = Vec::<BackgroundRun>::new();
    for cell in &frame.cells {
        if let Some(run) = runs.last_mut()
            && run.row == cell.row
            && run.end_column == cell.column
            && run.color == cell.background
        {
            run.end_column += 1;
        } else {
            runs.push(BackgroundRun {
                row: cell.row,
                start_column: cell.column,
                end_column: cell.column + 1,
                color: cell.background,
            });
        }
    }
    runs
}

fn range_columns(range: CellRange, row: usize, columns: usize) -> Option<(usize, usize)> {
    let range = range.normalized();
    if row < range.start.row || row > range.end.row || columns == 0 {
        return None;
    }
    let start = if row == range.start.row {
        range.start.column.min(columns - 1)
    } else {
        0
    };
    let end = if row == range.end.row {
        range.end.column.saturating_add(1).min(columns)
    } else {
        columns
    };
    (start < end).then_some((start, end))
}

fn cell_span_rect(
    row: usize,
    start_column: usize,
    end_column: usize,
    cell_width: f64,
    line_height: f64,
    scale: f64,
) -> Rect {
    Rect::new(
        column_boundary(start_column, cell_width, scale),
        row_boundary(row, line_height, scale),
        column_boundary(end_column, cell_width, scale),
        row_boundary(row + 1, line_height, scale),
    )
}

fn column_boundary(column: usize, cell_width: f64, scale: f64) -> f64 {
    snap_to_pixel(PADDING_X + column as f64 * cell_width, scale)
}

fn row_boundary(row: usize, line_height: f64, scale: f64) -> f64 {
    snap_to_pixel(PADDING_Y + row as f64 * line_height, scale)
}

fn snap_to_pixel(value: f64, scale: f64) -> f64 {
    (value * scale).round() / scale
}

fn find_cell_index(coordinate: f64, count: usize, boundary: impl Fn(usize) -> f64) -> usize {
    if count == 0 {
        return 0;
    }
    (0..count)
        .find(|index| coordinate < boundary(index + 1))
        .unwrap_or(count - 1)
}

fn color(value: TerminalColor) -> floem::peniko::Color {
    floem::peniko::Color::rgba8(value.red, value.green, value.blue, value.alpha)
}

#[cfg(test)]
mod tests {
    use matcha_terminal::{AlacrittyTerminal, TerminalModel, TerminalSize};

    use super::*;

    #[test]
    fn resolves_bundled_font_and_explicitly_falls_back() {
        register_bundled_fonts();
        let bundled = resolve_font_family(matcha_config::DEFAULT_FONT_FAMILY);
        assert_eq!(bundled.family, matcha_config::DEFAULT_FONT_FAMILY);
        assert!(!bundled.used_fallback);

        let missing = resolve_font_family("Matcha Definitely Missing Font");
        assert_eq!(missing.family, matcha_config::DEFAULT_FONT_FAMILY);
        assert!(missing.used_fallback);
    }

    #[test]
    fn default_metrics_are_compact_and_match_the_bundled_advance() {
        register_bundled_fonts();
        let (width, height) = cell_metrics(
            matcha_config::DEFAULT_FONT_SIZE,
            matcha_config::DEFAULT_FONT_FAMILY,
            matcha_config::DEFAULT_LINE_HEIGHT,
        );
        assert!((width - 8.4).abs() < 0.2, "unexpected cell width: {width}");
        assert!(
            (height - 16.1).abs() < 0.01,
            "unexpected cell height: {height}"
        );
    }

    #[test]
    fn row_builder_batches_simple_text_and_isolates_wide_cells() {
        register_bundled_fonts();
        let terminal = AlacrittyTerminal::new(TerminalSize::new(20, 2));
        terminal.feed("abc界e\u{301}".as_bytes());
        let state = TerminalPaintUpdate {
            frame: Arc::new(terminal.full_frame()),
            font_size: matcha_config::DEFAULT_FONT_SIZE,
            font_family: matcha_config::DEFAULT_FONT_FAMILY.into(),
            line_height: matcha_config::DEFAULT_LINE_HEIGHT,
            search: SearchResult::default(),
            preedit: String::new(),
            cursor_visible: true,
            background: TerminalColor::rgb(0, 0, 0),
        };
        let row = build_row(&state, 0);
        assert!(row.len() < state.frame.cells.len());
        assert!(row.iter().any(|glyph| glyph.columns == 2));
        assert!(row.iter().any(|glyph| glyph.columns >= 3));
    }

    #[test]
    fn merges_background_cells_into_contiguous_color_runs() {
        let terminal = AlacrittyTerminal::new(TerminalSize::new(6, 1));
        let mut frame = terminal.full_frame();
        for cell in &mut frame.cells {
            cell.background = if cell.column < 3 {
                TerminalColor::rgb(10, 20, 30)
            } else {
                TerminalColor::rgb(40, 50, 60)
            };
        }
        let runs = background_runs(&frame);
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].start_column, runs[0].end_column), (0, 3));
        assert_eq!((runs[1].start_column, runs[1].end_column), (3, 6));
    }

    #[test]
    fn snapped_cell_spans_share_exact_physical_boundaries() {
        for scale in [1.0, 1.25, 1.5] {
            let left = cell_span_rect(0, 0, 1, 8.4, 16.1, scale);
            let right = cell_span_rect(0, 1, 2, 8.4, 16.1, scale);
            let below = cell_span_rect(1, 0, 1, 8.4, 16.1, scale);
            assert_eq!(left.x1.to_bits(), right.x0.to_bits());
            assert_eq!(left.y1.to_bits(), below.y0.to_bits());
            assert!((left.x1 * scale).fract().abs() < f64::EPSILON);
            assert!((left.y1 * scale).fract().abs() < f64::EPSILON);
        }
    }

    #[test]
    fn selection_ranges_form_one_span_per_covered_row() {
        let range = CellRange {
            start: CellPoint { row: 2, column: 4 },
            end: CellPoint { row: 0, column: 2 },
        };
        assert_eq!(range_columns(range, 0, 10), Some((2, 10)));
        assert_eq!(range_columns(range, 1, 10), Some((0, 10)));
        assert_eq!(range_columns(range, 2, 10), Some((0, 5)));
        assert_eq!(range_columns(range, 3, 10), None);
    }
}
