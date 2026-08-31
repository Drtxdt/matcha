use crate::TerminalModes;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyCode {
    Character(String),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    Insert,
    Delete,
    PageUp,
    PageDown,
    Function(u8),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyInput {
    pub code: KeyCode,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseKind {
    Press,
    Release,
    Move,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseInput {
    pub kind: MouseKind,
    pub button: MouseButton,
    pub row: usize,
    pub column: usize,
    pub modifiers: Modifiers,
}

#[must_use]
pub fn encode_key(input: &KeyInput, modes: TerminalModes) -> Vec<u8> {
    if modes.kitty_keyboard
        && let Some(sequence) = kitty_sequence(input)
    {
        return sequence.into_bytes();
    }
    if let KeyCode::Character(text) = &input.code {
        let mut bytes = if input.modifiers.control {
            control_character(text).map_or_else(|| text.as_bytes().to_vec(), |byte| vec![byte])
        } else {
            text.as_bytes().to_vec()
        };
        if input.modifiers.alt {
            bytes.insert(0, 0x1b);
        }
        return bytes;
    }

    let modifier = modifier_parameter(input.modifiers);
    let sequence = match input.code {
        KeyCode::Enter => "\r".into(),
        KeyCode::Tab if input.modifiers.shift => "\x1b[Z".into(),
        KeyCode::Tab => "\t".into(),
        KeyCode::Backspace => "\x7f".into(),
        KeyCode::Escape => "\x1b".into(),
        KeyCode::Up => cursor_sequence('A', modifier, modes.application_cursor),
        KeyCode::Down => cursor_sequence('B', modifier, modes.application_cursor),
        KeyCode::Right => cursor_sequence('C', modifier, modes.application_cursor),
        KeyCode::Left => cursor_sequence('D', modifier, modes.application_cursor),
        KeyCode::Home => csi_final('H', modifier),
        KeyCode::End => csi_final('F', modifier),
        KeyCode::Insert => csi_tilde(2, modifier),
        KeyCode::Delete => csi_tilde(3, modifier),
        KeyCode::PageUp => csi_tilde(5, modifier),
        KeyCode::PageDown => csi_tilde(6, modifier),
        KeyCode::Function(number) => function_key(number, modifier),
        KeyCode::Character(_) => unreachable!("character input was handled above"),
    };
    let mut bytes = sequence.into_bytes();
    if input.modifiers.alt && !matches!(input.code, KeyCode::Escape) {
        bytes.insert(0, 0x1b);
    }
    bytes
}

#[must_use]
pub fn encode_mouse(input: MouseInput, modes: TerminalModes) -> Vec<u8> {
    if !modes.mouse_tracking {
        return Vec::new();
    }
    let mut code = match input.kind {
        MouseKind::WheelUp => 64,
        MouseKind::WheelDown => 65,
        MouseKind::Release => 3,
        MouseKind::Press | MouseKind::Move => match input.button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::None => 3,
        },
    };
    if input.kind == MouseKind::Move {
        code += 32;
    }
    code += u8::from(input.modifiers.shift) * 4
        + u8::from(input.modifiers.alt) * 8
        + u8::from(input.modifiers.control) * 16;

    if modes.sgr_mouse {
        let final_character = if input.kind == MouseKind::Release {
            'm'
        } else {
            'M'
        };
        format!(
            "\x1b[<{code};{};{}{final_character}",
            input.column + 1,
            input.row + 1,
        )
        .into_bytes()
    } else {
        vec![
            0x1b,
            b'[',
            b'M',
            code.saturating_add(32),
            u8::try_from(input.column + 33).unwrap_or(u8::MAX),
            u8::try_from(input.row + 33).unwrap_or(u8::MAX),
        ]
    }
}

#[must_use]
pub fn encode_paste(text: &str, modes: TerminalModes) -> Vec<u8> {
    if modes.bracketed_paste {
        let sanitized = text.replace("\x1b[201~", "");
        format!("\x1b[200~{sanitized}\x1b[201~").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

fn control_character(text: &str) -> Option<u8> {
    let character = text.chars().next()?;
    if text.chars().count() != 1 {
        return None;
    }
    match character.to_ascii_uppercase() {
        '@' | ' ' => Some(0),
        'A'..='Z' => Some(character.to_ascii_uppercase() as u8 - b'A' + 1),
        '[' => Some(27),
        '\\' => Some(28),
        ']' => Some(29),
        '^' => Some(30),
        '_' => Some(31),
        '?' => Some(127),
        _ => None,
    }
}

fn kitty_sequence(input: &KeyInput) -> Option<String> {
    let code = match &input.code {
        KeyCode::Character(text) => {
            if !input.modifiers.control && !input.modifiers.alt {
                return None;
            }
            u32::from(text.chars().next()?)
        }
        KeyCode::Enter => 13,
        KeyCode::Tab => 9,
        KeyCode::Backspace => 127,
        KeyCode::Escape => 27,
        KeyCode::Left => 57_350,
        KeyCode::Right => 57_351,
        KeyCode::Up => 57_352,
        KeyCode::Down => 57_353,
        KeyCode::Home => 57_360,
        KeyCode::End => 57_361,
        KeyCode::Insert => 57_362,
        KeyCode::Delete => 57_363,
        KeyCode::PageUp => 57_364,
        KeyCode::PageDown => 57_365,
        KeyCode::Function(number @ 1..=12) => 57_375 + u32::from(*number),
        KeyCode::Function(_) => return None,
    };
    Some(format!(
        "\x1b[{code};{}u",
        modifier_parameter(input.modifiers)
    ))
}

const fn modifier_parameter(modifiers: Modifiers) -> u8 {
    1 + modifiers.shift as u8 + 2 * modifiers.alt as u8 + 4 * modifiers.control as u8
}

fn cursor_sequence(final_character: char, modifier: u8, application: bool) -> String {
    if modifier == 1 {
        format!(
            "\x1b{}{final_character}",
            if application { 'O' } else { '[' }
        )
    } else {
        format!("\x1b[1;{modifier}{final_character}")
    }
}

fn csi_final(final_character: char, modifier: u8) -> String {
    if modifier == 1 {
        format!("\x1b[{final_character}")
    } else {
        format!("\x1b[1;{modifier}{final_character}")
    }
}

fn csi_tilde(number: u8, modifier: u8) -> String {
    if modifier == 1 {
        format!("\x1b[{number}~")
    } else {
        format!("\x1b[{number};{modifier}~")
    }
}

fn function_key(number: u8, modifier: u8) -> String {
    match number {
        1 => csi_final('P', modifier).replace('[', "O"),
        2 => csi_final('Q', modifier).replace('[', "O"),
        3 => csi_final('R', modifier).replace('[', "O"),
        4 => csi_final('S', modifier).replace('[', "O"),
        5 => csi_tilde(15, modifier),
        6 => csi_tilde(17, modifier),
        7 => csi_tilde(18, modifier),
        8 => csi_tilde(19, modifier),
        9 => csi_tilde(20, modifier),
        10 => csi_tilde(21, modifier),
        11 => csi_tilde(23, modifier),
        12 => csi_tilde(24, modifier),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyInput {
        KeyInput {
            code,
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn encodes_control_c_as_interrupt() {
        let mut input = key(KeyCode::Character("c".into()));
        input.modifiers.control = true;
        assert_eq!(encode_key(&input, TerminalModes::default()), vec![3]);
    }

    #[test]
    fn respects_application_cursor_mode() {
        assert_eq!(
            encode_key(&key(KeyCode::Up), TerminalModes::default()),
            b"\x1b[A"
        );
        assert_eq!(
            encode_key(
                &key(KeyCode::Up),
                TerminalModes {
                    application_cursor: true,
                    ..TerminalModes::default()
                },
            ),
            b"\x1bOA"
        );
    }

    #[test]
    fn wraps_and_sanitizes_bracketed_paste() {
        let encoded = encode_paste(
            "hello\x1b[201~world",
            TerminalModes {
                bracketed_paste: true,
                ..TerminalModes::default()
            },
        );
        assert_eq!(encoded, b"\x1b[200~helloworld\x1b[201~");
    }

    #[test]
    fn encodes_sgr_mouse_clicks() {
        let bytes = encode_mouse(
            MouseInput {
                kind: MouseKind::Press,
                button: MouseButton::Left,
                row: 4,
                column: 9,
                modifiers: Modifiers::default(),
            },
            TerminalModes {
                mouse_tracking: true,
                sgr_mouse: true,
                ..TerminalModes::default()
            },
        );
        assert_eq!(bytes, b"\x1b[<0;10;5M");
    }

    #[test]
    fn encodes_kitty_modified_keys() {
        let input = KeyInput {
            code: KeyCode::Character("c".into()),
            modifiers: Modifiers {
                control: true,
                ..Modifiers::default()
            },
        };
        assert_eq!(
            encode_key(
                &input,
                TerminalModes {
                    kitty_keyboard: true,
                    ..TerminalModes::default()
                }
            ),
            b"\x1b[99;5u"
        );
    }
}
