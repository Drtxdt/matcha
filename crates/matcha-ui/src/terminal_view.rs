#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::any::Any;
use std::sync::Arc;

use floem::context::{PaintCx, UpdateCx};
use floem::peniko::kurbo::{Point, Rect, Stroke};
use floem::reactive::{RwSignal, SignalGet, create_effect};
use floem::text::FONT_SYSTEM;
use floem::text::{Attrs, AttrsList, FamilyOwned, Style, TextLayout, Weight};
use floem::{View, ViewId};
use floem_renderer::Renderer;
use matcha_terminal::{
    CellPoint, CursorShape, SearchResult, TerminalColor, TerminalFrame, UnderlineStyle,
};

const PADDING_X: f64 = 8.0;
const PADDING_Y: f64 = 6.0;

#[derive(Clone)]
struct TerminalPaintUpdate {
    frame: Arc<TerminalFrame>,
    font_size: f32,
    font_family: String,
    search: SearchResult,
    preedit: String,
    cursor_visible: bool,
    background: TerminalColor,
}

pub struct TerminalView {
    id: ViewId,
    state: TerminalPaintUpdate,
    rows: Vec<Vec<CachedGlyph>>,
}

struct CachedGlyph {
    column: usize,
    layout: TextLayout,
    underline: UnderlineStyle,
    underline_color: TerminalColor,
}

pub fn register_bundled_fonts() {
    const FONTS: [&[u8]; 4] = [
        include_bytes!("../assets/fonts/JetBrainsMonoNL-Regular.ttf"),
        include_bytes!("../assets/fonts/JetBrainsMonoNL-Bold.ttf"),
        include_bytes!("../assets/fonts/JetBrainsMonoNL-Italic.ttf"),
        include_bytes!("../assets/fonts/JetBrainsMonoNL-BoldItalic.ttf"),
    ];
    let mut font_system = FONT_SYSTEM.lock();
    for font in FONTS {
        font_system.db_mut().load_font_data(font.to_vec());
    }
}

pub fn terminal_view(
    frame: RwSignal<Arc<TerminalFrame>>,
    font_size: RwSignal<f32>,
    font_family: RwSignal<String>,
    search: RwSignal<SearchResult>,
    preedit: RwSignal<String>,
    cursor_visible: RwSignal<bool>,
    background: RwSignal<TerminalColor>,
) -> TerminalView {
    let id = ViewId::new();
    let initial = TerminalPaintUpdate {
        frame: frame.get_untracked(),
        font_size: font_size.get_untracked(),
        font_family: font_family.get_untracked(),
        search: search.get_untracked(),
        preedit: preedit.get_untracked(),
        cursor_visible: cursor_visible.get_untracked(),
        background: background.get_untracked(),
    };
    create_effect(move |_| {
        id.update_state(TerminalPaintUpdate {
            frame: frame.get(),
            font_size: font_size.get(),
            font_family: font_family.get(),
            search: search.get(),
            preedit: preedit.get(),
            cursor_visible: cursor_visible.get(),
            background: background.get(),
        });
    });
    let mut view = TerminalView {
        id,
        state: initial,
        rows: Vec::new(),
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
                || self.state.font_family != state.font_family;
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
        let (cell_width, line_height) = cell_metrics(font_size, &self.state.font_family);
        let background = color(self.state.background);
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

        for cell in &frame.cells {
            let rect = cell_rect(cell.row, cell.column, cell_width, line_height);
            cx.fill(&rect, color(cell.background), 0.0);
        }

        if let Some(selection) = frame.selection {
            for row in 0..frame.size.lines {
                for column in 0..frame.size.columns {
                    if selection.contains(CellPoint { row, column }) {
                        cx.fill(
                            &cell_rect(row, column, cell_width, line_height),
                            floem::peniko::Color::rgba8(82, 112, 91, 190),
                            0.0,
                        );
                    }
                }
            }
        }

        for (index, range) in self.state.search.matches.iter().enumerate() {
            let search_color = if self.state.search.active == Some(index) {
                floem::peniko::Color::rgba8(215, 176, 74, 220)
            } else {
                floem::peniko::Color::rgba8(133, 112, 49, 170)
            };
            for column in range.start.column..=range.end.column {
                cx.fill(
                    &cell_rect(range.start.row, column, cell_width, line_height),
                    search_color,
                    0.0,
                );
            }
        }

        for (row, glyphs) in self.rows.iter().enumerate() {
            for glyph in glyphs {
                cx.draw_text(
                    &glyph.layout,
                    Point::new(
                        PADDING_X + glyph.column as f64 * cell_width,
                        PADDING_Y + row as f64 * line_height,
                    ),
                );
                if glyph.underline == UnderlineStyle::None {
                    continue;
                }
                let y = PADDING_Y + (row + 1) as f64 * line_height - 2.0;
                cx.stroke(
                    &floem::peniko::kurbo::Line::new(
                        (PADDING_X + glyph.column as f64 * cell_width, y),
                        (PADDING_X + (glyph.column + 1) as f64 * cell_width, y),
                    ),
                    color(glyph.underline_color),
                    &Stroke::new(1.0),
                );
            }
        }

        if self.state.cursor_visible {
            paint_cursor(cx, frame, cell_width, line_height);
        }
        if !self.state.preedit.is_empty() {
            paint_preedit(
                cx,
                frame,
                &self.state.preedit,
                font_size,
                cell_width,
                line_height,
            );
        }
    }
}

fn build_row(state: &TerminalPaintUpdate, row: usize) -> Vec<CachedGlyph> {
    let family = [
        FamilyOwned::Name(state.font_family.clone()),
        FamilyOwned::Name("JetBrains Mono NL".into()),
        FamilyOwned::Monospace,
    ];
    state
        .frame
        .cells
        .iter()
        .filter(|cell| cell.row == row && !cell.style.wide_spacer && cell.text != " ")
        .map(|cell| {
            let mut attrs = Attrs::new()
                .font_size(state.font_size)
                .family(&family)
                .color(color(cell.foreground));
            if cell.style.bold {
                attrs = attrs.weight(Weight::BOLD);
            }
            if cell.style.italic {
                attrs = attrs.style(Style::Italic);
            }
            let mut layout = TextLayout::new();
            layout.set_text(&cell.text, AttrsList::new(attrs));
            CachedGlyph {
                column: cell.column,
                layout,
                underline: cell.style.underline,
                underline_color: cell.underline_color,
            }
        })
        .collect()
}

pub fn cell_metrics(font_size: f32, font_family: &str) -> (f64, f64) {
    let family = [
        FamilyOwned::Name(font_family.to_owned()),
        FamilyOwned::Name("JetBrains Mono NL".into()),
        FamilyOwned::Monospace,
    ];
    let mut layout = TextLayout::new();
    layout.set_text(
        "M",
        AttrsList::new(Attrs::new().font_size(font_size).family(&family)),
    );
    let width = layout
        .layout_runs()
        .next()
        .map_or(f64::from(font_size) * 0.62, |run| f64::from(run.line_w));
    (width.max(1.0), f64::from(font_size) * 1.4)
}

pub fn point_to_cell(
    point: Point,
    font_size: f32,
    font_family: &str,
    frame: &TerminalFrame,
) -> CellPoint {
    let (cell_width, line_height) = cell_metrics(font_size, font_family);
    let column = ((point.x - PADDING_X).max(0.0) / cell_width).floor() as usize;
    let row = ((point.y - PADDING_Y).max(0.0) / line_height).floor() as usize;
    CellPoint {
        row: row.min(frame.size.lines.saturating_sub(1)),
        column: column.min(frame.size.columns.saturating_sub(1)),
    }
}

fn paint_cursor(cx: &mut PaintCx, frame: &TerminalFrame, cell_width: f64, line_height: f64) {
    let cursor = frame.cursor;
    let rect = cell_rect(cursor.row, cursor.column, cell_width, line_height);
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
            PADDING_X + frame.cursor.column as f64 * cell_width,
            PADDING_Y + frame.cursor.row as f64 * line_height,
        ),
    );
}

fn cell_rect(row: usize, column: usize, cell_width: f64, line_height: f64) -> Rect {
    let x = PADDING_X + column as f64 * cell_width;
    let y = PADDING_Y + row as f64 * line_height;
    Rect::new(x, y, x + cell_width, y + line_height)
}

fn color(value: TerminalColor) -> floem::peniko::Color {
    floem::peniko::Color::rgba8(value.red, value.green, value.blue, value.alpha)
}
