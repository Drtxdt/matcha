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
    search: SearchResult,
    preedit: String,
}

pub struct TerminalView {
    id: ViewId,
    state: TerminalPaintUpdate,
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
    search: RwSignal<SearchResult>,
    preedit: RwSignal<String>,
) -> TerminalView {
    let id = ViewId::new();
    let initial = TerminalPaintUpdate {
        frame: frame.get_untracked(),
        font_size: font_size.get_untracked(),
        search: search.get_untracked(),
        preedit: preedit.get_untracked(),
    };
    create_effect(move |_| {
        id.update_state(TerminalPaintUpdate {
            frame: frame.get(),
            font_size: font_size.get(),
            search: search.get(),
            preedit: preedit.get(),
        });
    });
    TerminalView { id, state: initial }
}

impl View for TerminalView {
    fn id(&self) -> ViewId {
        self.id
    }

    fn update(&mut self, _cx: &mut UpdateCx, state: Box<dyn Any>) {
        if let Ok(state) = state.downcast::<TerminalPaintUpdate>() {
            self.state = *state;
            self.id.request_paint();
        }
    }

    fn paint(&mut self, cx: &mut PaintCx) {
        let frame = &self.state.frame;
        let font_size = self.state.font_size;
        let cell_width = cell_width(font_size);
        let line_height = line_height(font_size);
        let background = color(matcha_terminal::TerminalColor::rgb(18, 24, 20));
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

        let family = [
            FamilyOwned::Name("JetBrains Mono NL".into()),
            FamilyOwned::Monospace,
        ];
        for cell in &frame.cells {
            if cell.style.wide_spacer || cell.text == " " {
                continue;
            }
            let mut attrs = Attrs::new()
                .font_size(font_size)
                .family(&family)
                .color(color(cell.foreground));
            if cell.style.bold {
                attrs = attrs.weight(Weight::BOLD);
            }
            if cell.style.italic {
                attrs = attrs.style(Style::Italic);
            }
            let mut text = TextLayout::new();
            text.set_text(&cell.text, AttrsList::new(attrs));
            cx.draw_text(
                &text,
                Point::new(
                    PADDING_X + cell.column as f64 * cell_width,
                    PADDING_Y + cell.row as f64 * line_height,
                ),
            );

            if cell.style.underline != UnderlineStyle::None {
                let y = PADDING_Y + (cell.row + 1) as f64 * line_height - 2.0;
                cx.stroke(
                    &floem::peniko::kurbo::Line::new(
                        (PADDING_X + cell.column as f64 * cell_width, y),
                        (PADDING_X + (cell.column + 1) as f64 * cell_width, y),
                    ),
                    color(cell.underline_color),
                    &Stroke::new(1.0),
                );
            }
        }

        paint_cursor(cx, frame, cell_width, line_height);
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

pub fn cell_width(font_size: f32) -> f64 {
    f64::from(font_size) * 0.62
}

pub fn line_height(font_size: f32) -> f64 {
    f64::from(font_size) * 1.4
}

pub fn point_to_cell(point: Point, font_size: f32, frame: &TerminalFrame) -> CellPoint {
    let column = ((point.x - PADDING_X).max(0.0) / cell_width(font_size)).floor() as usize;
    let row = ((point.y - PADDING_Y).max(0.0) / line_height(font_size)).floor() as usize;
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
