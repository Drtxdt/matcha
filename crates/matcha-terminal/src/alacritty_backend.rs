use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{ClipboardType, Config, Term, TermMode};
use alacritty_terminal::vte::ansi;
use alacritty_terminal::vte::ansi::{Color, CursorShape as AlacrittyCursorShape, NamedColor, Rgb};
use parking_lot::Mutex;

use crate::{
    CellPoint, CellRange, CellStyle, ClipboardKind, CursorShape, FrameDamage, RenderCell,
    SearchRequest, SearchResult, TerminalAction, TerminalColor, TerminalCursor, TerminalFrame,
    TerminalModel, TerminalModes, TerminalSize, UnderlineStyle,
};

#[derive(Clone, Debug)]
struct ModelEventListener {
    actions: Arc<Mutex<VecDeque<TerminalAction>>>,
}

impl EventListener for ModelEventListener {
    fn send_event(&self, event: Event) {
        let action = match event {
            Event::PtyWrite(response) => TerminalAction::WriteToHost(response.into_bytes()),
            Event::Title(title) => TerminalAction::Title(title),
            Event::ResetTitle => TerminalAction::ResetTitle,
            Event::Bell => TerminalAction::Bell,
            Event::Wakeup | Event::CursorBlinkingChange | Event::MouseCursorDirty => {
                TerminalAction::Wakeup
            }
            Event::Exit => TerminalAction::ExitRequested,
            Event::ClipboardStore(kind, text) => TerminalAction::ClipboardStore {
                kind: clipboard_kind(kind),
                text,
            },
            Event::ClipboardLoad(kind, formatter) => TerminalAction::ClipboardLoad {
                kind: clipboard_kind(kind),
                formatter,
            },
            Event::ColorRequest(..) | Event::TextAreaSizeRequest(..) | Event::ChildExit(..) => {
                return;
            }
        };
        self.actions.lock().push_back(action);
    }
}

#[derive(Clone, Copy, Debug)]
struct GridSize(TerminalSize);

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.0.lines
    }

    fn screen_lines(&self) -> usize {
        self.0.lines
    }

    fn columns(&self) -> usize {
        self.0.columns
    }
}

pub struct AlacrittyTerminal {
    term: Arc<Mutex<Term<ModelEventListener>>>,
    actions: Arc<Mutex<VecDeque<TerminalAction>>>,
    selection: Arc<Mutex<Option<CellRange>>>,
    revision: AtomicU64,
}

impl AlacrittyTerminal {
    #[must_use]
    pub fn new(size: TerminalSize) -> Self {
        Self::new_with_scrollback(size, 50_000)
    }

    #[must_use]
    pub fn new_with_scrollback(size: TerminalSize, scrollback_lines: usize) -> Self {
        let config = Config {
            scrolling_history: scrollback_lines,
            ..Config::default()
        };
        let actions = Arc::new(Mutex::new(VecDeque::new()));
        let listener = ModelEventListener {
            actions: Arc::clone(&actions),
        };
        let term = Term::new(config, &GridSize(size), listener);
        Self {
            term: Arc::new(Mutex::new(term)),
            actions,
            selection: Arc::new(Mutex::new(None)),
            revision: AtomicU64::new(0),
        }
    }
}

impl TerminalModel for AlacrittyTerminal {
    fn feed(&self, bytes: &[u8]) {
        let mut processor: ansi::Processor<ansi::StdSyncHandler> = ansi::Processor::new();
        processor.advance(&mut *self.term.lock(), bytes);
        self.revision.fetch_add(1, Ordering::Release);
    }

    fn resize(&self, size: TerminalSize) {
        self.term.lock().resize(GridSize(size));
        self.revision.fetch_add(1, Ordering::Release);
    }

    fn frame(&self) -> TerminalFrame {
        let mut term = self.term.lock();
        let size = TerminalSize::new(term.columns(), term.screen_lines());
        let content = term.renderable_content();
        let display_offset = content.display_offset;
        let modes = terminal_modes(content.mode);
        let display_offset_i32 = i32::try_from(display_offset).unwrap_or(i32::MAX);
        let cursor_line = content
            .cursor
            .point
            .line
            .0
            .saturating_add(display_offset_i32);
        let cursor = TerminalCursor {
            row: usize::try_from(cursor_line).unwrap_or(0),
            column: content.cursor.point.column.0,
            shape: cursor_shape(content.cursor.shape),
        };
        let colors = *content.colors;
        let cells = content
            .display_iter
            .filter_map(|indexed| {
                let row = usize::try_from(indexed.point.line.0.saturating_add(display_offset_i32))
                    .ok()?;
                (row < size.lines)
                    .then(|| render_cell(row, indexed.point.column.0, indexed.cell, &colors))
            })
            .collect();
        term.reset_damage();

        TerminalFrame {
            revision: self.revision.load(Ordering::Acquire),
            size,
            damage: FrameDamage::Full,
            cells,
            cursor,
            selection: *self.selection.lock(),
            display_offset,
            modes,
        }
    }

    fn size(&self) -> TerminalSize {
        let term = self.term.lock();
        TerminalSize::new(term.columns(), term.screen_lines())
    }

    fn drain_actions(&self) -> Vec<TerminalAction> {
        self.actions.lock().drain(..).collect()
    }

    fn drain_host_responses(&self) -> Vec<Vec<u8>> {
        let mut queue = self.actions.lock();
        let mut retained = VecDeque::new();
        let mut responses = Vec::new();
        while let Some(action) = queue.pop_front() {
            match action {
                TerminalAction::WriteToHost(bytes) => responses.push(bytes),
                action => retained.push_back(action),
            }
        }
        *queue = retained;
        responses
    }

    fn scroll(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
        self.revision.fetch_add(1, Ordering::Release);
    }

    fn set_scrollback_limit(&self, lines: usize) {
        self.term.lock().set_options(Config {
            scrolling_history: lines,
            ..Config::default()
        });
        self.revision.fetch_add(1, Ordering::Release);
    }

    fn set_selection(&self, selection: Option<CellRange>) {
        *self.selection.lock() = selection.map(CellRange::normalized);
        self.revision.fetch_add(1, Ordering::Release);
    }

    fn selection_text(&self) -> Option<String> {
        let range = self.selection.lock().as_ref().copied()?.normalized();
        let frame = self.frame();
        let mut output = String::new();
        for row in range.start.row..=range.end.row.min(frame.size.lines.saturating_sub(1)) {
            let start = if row == range.start.row {
                range.start.column
            } else {
                0
            };
            let end = if row == range.end.row {
                range.end.column
            } else {
                frame.size.columns.saturating_sub(1)
            };
            let mut line = String::new();
            for cell in frame
                .cells
                .iter()
                .filter(|cell| cell.row == row && cell.column >= start && cell.column <= end)
            {
                if !cell.style.wide_spacer {
                    line.push_str(&cell.text);
                }
            }
            output.push_str(line.trim_end());
            if row < range.end.row {
                output.push('\n');
            }
        }
        Some(output)
    }

    fn search(&self, request: &SearchRequest) -> SearchResult {
        if request.query.is_empty() {
            return SearchResult::default();
        }
        let frame = self.frame();
        let needle = if request.case_sensitive {
            request.query.clone()
        } else {
            request.query.to_lowercase()
        };
        let needle_length = request.query.chars().count().max(1);
        let mut matches = Vec::new();
        for row in 0..frame.size.lines {
            let line: String = frame
                .cells
                .iter()
                .filter(|cell| cell.row == row && !cell.style.wide_spacer)
                .flat_map(|cell| cell.text.chars())
                .collect();
            let haystack = if request.case_sensitive {
                line
            } else {
                line.to_lowercase()
            };
            for (byte_index, _) in haystack.match_indices(&needle) {
                let column = haystack[..byte_index].chars().count();
                matches.push(CellRange {
                    start: CellPoint { row, column },
                    end: CellPoint {
                        row,
                        column: column + needle_length - 1,
                    },
                });
            }
        }
        SearchResult {
            active: (!matches.is_empty()).then_some(0),
            matches,
        }
    }
}

fn render_cell(row: usize, column: usize, cell: &Cell, colors: &Colors) -> RenderCell {
    let style = cell_style(cell.flags);
    let mut foreground = resolve_color(cell.fg, colors, true);
    let mut background = resolve_color(cell.bg, colors, false);
    if style.inverse {
        std::mem::swap(&mut foreground, &mut background);
    }
    let mut text = cell.c.to_string();
    if let Some(zerowidth) = cell.zerowidth() {
        text.extend(zerowidth);
    }
    if style.hidden {
        text = " ".into();
    }
    RenderCell {
        row,
        column,
        text,
        foreground,
        background,
        underline_color: cell
            .underline_color()
            .map_or(foreground, |color| resolve_color(color, colors, true)),
        style,
        hyperlink: cell.hyperlink().map(|link| link.uri().to_owned()),
    }
}

fn cell_style(flags: Flags) -> CellStyle {
    let underline = if flags.contains(Flags::DOUBLE_UNDERLINE) {
        UnderlineStyle::Double
    } else if flags.contains(Flags::UNDERCURL) {
        UnderlineStyle::Curl
    } else if flags.contains(Flags::DOTTED_UNDERLINE) {
        UnderlineStyle::Dotted
    } else if flags.contains(Flags::DASHED_UNDERLINE) {
        UnderlineStyle::Dashed
    } else if flags.contains(Flags::UNDERLINE) {
        UnderlineStyle::Single
    } else {
        UnderlineStyle::None
    };
    CellStyle {
        bold: flags.contains(Flags::BOLD),
        italic: flags.contains(Flags::ITALIC),
        dim: flags.contains(Flags::DIM),
        inverse: flags.contains(Flags::INVERSE),
        hidden: flags.contains(Flags::HIDDEN),
        strikeout: flags.contains(Flags::STRIKEOUT),
        underline,
        wide: flags.contains(Flags::WIDE_CHAR),
        wide_spacer: flags.contains(Flags::WIDE_CHAR_SPACER),
    }
}

fn terminal_modes(mode: TermMode) -> TerminalModes {
    TerminalModes {
        application_cursor: mode.contains(TermMode::APP_CURSOR),
        alternate_screen: mode.contains(TermMode::ALT_SCREEN),
        bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
        mouse_tracking: mode.intersects(
            TermMode::MOUSE_REPORT_CLICK
                | TermMode::MOUSE_DRAG
                | TermMode::MOUSE_MOTION
                | TermMode::MOUSE_MODE,
        ),
        sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
        kitty_keyboard: mode.contains(TermMode::KITTY_KEYBOARD_PROTOCOL),
    }
}

fn clipboard_kind(kind: ClipboardType) -> ClipboardKind {
    match kind {
        ClipboardType::Clipboard => ClipboardKind::Clipboard,
        ClipboardType::Selection => ClipboardKind::Selection,
    }
}

fn cursor_shape(shape: AlacrittyCursorShape) -> CursorShape {
    match shape {
        AlacrittyCursorShape::Block | AlacrittyCursorShape::HollowBlock => CursorShape::Block,
        AlacrittyCursorShape::Underline => CursorShape::Underline,
        AlacrittyCursorShape::Beam => CursorShape::Beam,
        AlacrittyCursorShape::Hidden => CursorShape::Hidden,
    }
}

fn resolve_color(color: Color, colors: &Colors, foreground: bool) -> TerminalColor {
    let rgb = match color {
        Color::Spec(rgb) => rgb,
        Color::Indexed(index) => colors[index as usize].unwrap_or_else(|| indexed_color(index)),
        Color::Named(named) => colors[named].unwrap_or_else(|| named_color(named, foreground)),
    };
    TerminalColor::rgb(rgb.r, rgb.g, rgb.b)
}

fn named_color(color: NamedColor, foreground: bool) -> Rgb {
    let index = color as usize;
    if index < 16 {
        indexed_color(u8::try_from(index).expect("ANSI named color index must fit in u8"))
    } else if color == NamedColor::Background || !foreground {
        Rgb {
            r: 18,
            g: 24,
            b: 20,
        }
    } else if color == NamedColor::Cursor {
        Rgb {
            r: 167,
            g: 213,
            b: 165,
        }
    } else {
        Rgb {
            r: 220,
            g: 229,
            b: 220,
        }
    }
}

#[allow(clippy::too_many_lines)]
fn indexed_color(index: u8) -> Rgb {
    const ANSI: [Rgb; 16] = [
        Rgb {
            r: 24,
            g: 30,
            b: 26,
        },
        Rgb {
            r: 230,
            g: 99,
            b: 92,
        },
        Rgb {
            r: 116,
            g: 186,
            b: 120,
        },
        Rgb {
            r: 222,
            g: 185,
            b: 91,
        },
        Rgb {
            r: 103,
            g: 155,
            b: 222,
        },
        Rgb {
            r: 189,
            g: 126,
            b: 204,
        },
        Rgb {
            r: 91,
            g: 190,
            b: 183,
        },
        Rgb {
            r: 210,
            g: 218,
            b: 211,
        },
        Rgb {
            r: 103,
            g: 113,
            b: 105,
        },
        Rgb {
            r: 245,
            g: 128,
            b: 120,
        },
        Rgb {
            r: 151,
            g: 211,
            b: 153,
        },
        Rgb {
            r: 239,
            g: 207,
            b: 116,
        },
        Rgb {
            r: 135,
            g: 179,
            b: 236,
        },
        Rgb {
            r: 211,
            g: 155,
            b: 222,
        },
        Rgb {
            r: 124,
            g: 213,
            b: 207,
        },
        Rgb {
            r: 240,
            g: 244,
            b: 240,
        },
    ];
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let value = index - 16;
            let component = |part: u8| if part == 0 { 0 } else { 55 + part * 40 };
            Rgb {
                r: component(value / 36),
                g: component((value % 36) / 6),
                b: component(value % 6),
            }
        }
        _ => {
            let gray = 8 + (index - 232) * 10;
            Rgb {
                r: gray,
                g: gray,
                b: gray,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_and_ansi_attributes() {
        let terminal = AlacrittyTerminal::new(TerminalSize::new(20, 3));
        terminal.feed(b"plain \x1b[1;31mred\x1b[0m");
        assert_eq!(terminal.visible_text(), "plain red");
        let red = terminal
            .frame()
            .cells
            .into_iter()
            .find(|cell| cell.column == 6)
            .expect("styled cell should exist");
        assert!(red.style.bold);
        assert_ne!(red.foreground, TerminalColor::default());
    }

    #[test]
    fn handles_cursor_movement() {
        let terminal = AlacrittyTerminal::new(TerminalSize::new(10, 3));
        terminal.feed(b"abc\rZ");
        assert_eq!(terminal.visible_text(), "Zbc");
    }

    #[test]
    fn exposes_device_status_replies_for_the_host() {
        let terminal = AlacrittyTerminal::new(TerminalSize::new(80, 24));
        terminal.feed(b"\x1b[6n");
        assert_eq!(terminal.drain_host_responses(), vec![b"\x1b[1;1R"]);
    }

    #[test]
    fn selects_and_searches_visible_text() {
        let terminal = AlacrittyTerminal::new(TerminalSize::new(20, 3));
        terminal.feed(b"Matcha terminal");
        terminal.set_selection(Some(CellRange {
            start: CellPoint { row: 0, column: 0 },
            end: CellPoint { row: 0, column: 5 },
        }));
        assert_eq!(terminal.selection_text().as_deref(), Some("Matcha"));
        let result = terminal.search(&SearchRequest {
            query: "TERMINAL".into(),
            case_sensitive: false,
        });
        assert_eq!(result.matches.len(), 1);
    }
}
