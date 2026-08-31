//! A replaceable terminal-emulation boundary.

mod alacritty_backend;
mod input;

use std::fmt;
use std::sync::Arc;

pub use alacritty_backend::AlacrittyTerminal;
pub use input::{
    KeyCode, KeyInput, Modifiers, MouseButton, MouseInput, MouseKind, encode_key, encode_mouse,
    encode_paste,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalSize {
    pub columns: usize,
    pub lines: usize,
}

impl TerminalSize {
    #[must_use]
    pub const fn new(columns: usize, lines: usize) -> Self {
        Self { columns, lines }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerminalColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl TerminalColor {
    #[must_use]
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha: 255,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Curl,
    Dotted,
    Dashed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct CellStyle {
    pub bold: bool,
    pub italic: bool,
    pub dim: bool,
    pub inverse: bool,
    pub hidden: bool,
    pub strikeout: bool,
    pub underline: UnderlineStyle,
    pub wide: bool,
    pub wide_spacer: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderCell {
    pub row: usize,
    pub column: usize,
    pub text: String,
    pub foreground: TerminalColor,
    pub background: TerminalColor,
    pub underline_color: TerminalColor,
    pub style: CellStyle,
    pub hyperlink: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorShape {
    Block,
    Underline,
    Beam,
    Hidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalCursor {
    pub row: usize,
    pub column: usize,
    pub shape: CursorShape,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CellPoint {
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellRange {
    pub start: CellPoint,
    pub end: CellPoint,
}

impl CellRange {
    #[must_use]
    pub fn normalized(self) -> Self {
        if (self.start.row, self.start.column) <= (self.end.row, self.end.column) {
            self
        } else {
            Self {
                start: self.end,
                end: self.start,
            }
        }
    }

    #[must_use]
    pub fn contains(self, point: CellPoint) -> bool {
        let range = self.normalized();
        (point.row, point.column) >= (range.start.row, range.start.column)
            && (point.row, point.column) <= (range.end.row, range.end.column)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FrameDamage {
    #[default]
    Full,
    Partial,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct TerminalModes {
    pub application_cursor: bool,
    pub alternate_screen: bool,
    pub bracketed_paste: bool,
    pub mouse_tracking: bool,
    pub sgr_mouse: bool,
    pub kitty_keyboard: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalFrame {
    pub revision: u64,
    pub size: TerminalSize,
    pub damage: FrameDamage,
    pub cells: Vec<RenderCell>,
    pub cursor: TerminalCursor,
    pub selection: Option<CellRange>,
    pub display_offset: usize,
    pub modes: TerminalModes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardKind {
    Clipboard,
    Selection,
}

#[derive(Clone)]
pub enum TerminalAction {
    WriteToHost(Vec<u8>),
    Title(String),
    ResetTitle,
    Bell,
    Wakeup,
    ExitRequested,
    ClipboardStore {
        kind: ClipboardKind,
        text: String,
    },
    ClipboardLoad {
        kind: ClipboardKind,
        formatter: Arc<dyn Fn(&str) -> String + Send + Sync>,
    },
}

impl fmt::Debug for TerminalAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WriteToHost(bytes) => formatter.debug_tuple("WriteToHost").field(bytes).finish(),
            Self::Title(title) => formatter.debug_tuple("Title").field(title).finish(),
            Self::ResetTitle => formatter.write_str("ResetTitle"),
            Self::Bell => formatter.write_str("Bell"),
            Self::Wakeup => formatter.write_str("Wakeup"),
            Self::ExitRequested => formatter.write_str("ExitRequested"),
            Self::ClipboardStore { kind, text } => formatter
                .debug_struct("ClipboardStore")
                .field("kind", kind)
                .field("text", text)
                .finish(),
            Self::ClipboardLoad { kind, .. } => formatter
                .debug_struct("ClipboardLoad")
                .field("kind", kind)
                .finish_non_exhaustive(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchRequest {
    pub query: String,
    pub case_sensitive: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchResult {
    pub matches: Vec<CellRange>,
    pub active: Option<usize>,
}

pub trait TerminalModel: Send + Sync {
    fn feed(&self, bytes: &[u8]);
    fn resize(&self, size: TerminalSize);
    fn frame(&self) -> TerminalFrame;
    fn size(&self) -> TerminalSize;
    fn drain_actions(&self) -> Vec<TerminalAction>;
    fn scroll(&self, lines: i32);
    fn set_scrollback_limit(&self, lines: usize);
    fn set_selection(&self, selection: Option<CellRange>);
    fn selection_text(&self) -> Option<String>;
    fn search(&self, request: &SearchRequest) -> SearchResult;

    fn drain_host_responses(&self) -> Vec<Vec<u8>> {
        self.drain_actions()
            .into_iter()
            .filter_map(|action| match action {
                TerminalAction::WriteToHost(bytes) => Some(bytes),
                _ => None,
            })
            .collect()
    }

    #[must_use]
    fn visible_text(&self) -> String {
        let frame = self.frame();
        let mut output = String::new();
        for row in 0..frame.size.lines {
            let mut line = String::new();
            for cell in frame.cells.iter().filter(|cell| cell.row == row) {
                if !cell.style.wide_spacer {
                    line.push_str(&cell.text);
                }
            }
            output.push_str(line.trim_end());
            if row + 1 < frame.size.lines {
                output.push('\n');
            }
        }
        output.trim_end_matches('\n').to_owned()
    }
}
