//! Floem-specific Matcha workstation UI.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

mod terminal_view;

use std::sync::Arc;
use std::thread;

use crossbeam_channel::{Sender, unbounded};
use floem::Clipboard;
use floem::action::set_ime_allowed;
use floem::event::{Event, EventListener};
use floem::ext_event::create_signal_from_channel;
use floem::keyboard::{Key, NamedKey};
use floem::prelude::*;
use floem::reactive::{RwSignal, SignalGet, SignalUpdate, create_effect, create_rw_signal};
use matcha_config::{AppConfig, ConfigLoad, LocalePreference, ShellProfileConfig, ThemePreference};
use matcha_core::ShellProfile;
use matcha_pty::{LocalPtySession, SessionEvent};
use matcha_terminal::{
    AlacrittyTerminal, CellPoint, CellRange, KeyCode, KeyInput, Modifiers,
    MouseButton as TerminalMouseButton, MouseInput, MouseKind, SearchRequest, SearchResult,
    TerminalAction, TerminalFrame, TerminalModel, TerminalSize, encode_key, encode_mouse,
    encode_paste,
};
use parking_lot::Mutex;

use crate::terminal_view::{line_height, point_to_cell, terminal_view};

const INITIAL_COLUMNS: usize = 100;
const INITIAL_LINES: usize = 30;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionStatus {
    Starting,
    Running,
    Exited(u32),
    Failed,
}

struct WorkspaceState {
    terminal: Arc<AlacrittyTerminal>,
    terminal_model: Arc<dyn TerminalModel>,
    session: Mutex<Option<LocalPtySession>>,
    profile: Mutex<ShellProfile>,
    profile_id: String,
    profile_name: String,
    event_tx: Sender<SessionEvent>,
    config: Mutex<AppConfig>,
    config_path: std::path::PathBuf,
}

pub fn launch() {
    terminal_view::register_bundled_fonts();
    let loaded = match matcha_config::load() {
        Ok(loaded) => loaded,
        Err(error) => {
            tracing::error!(%error, "failed to load configuration");
            ConfigLoad {
                config: AppConfig::default(),
                warning: Some(error.to_string()),
                path: matcha_config::config_path()
                    .unwrap_or_else(|| std::path::PathBuf::from("config.toml")),
            }
        }
    };
    floem::launch(move || app_view(loaded));
}

fn app_view(loaded: ConfigLoad) -> impl IntoView {
    let config = loaded.config;
    let profile = config
        .default_shell()
        .cloned()
        .unwrap_or_else(fallback_profile);
    let terminal = Arc::new(AlacrittyTerminal::new_with_scrollback(
        TerminalSize::new(INITIAL_COLUMNS, INITIAL_LINES),
        config.terminal.scrollback_lines,
    ));
    let terminal_model: Arc<dyn TerminalModel> = terminal.clone();
    let (event_tx, event_rx) = unbounded();
    let state = Arc::new(WorkspaceState {
        terminal: Arc::clone(&terminal),
        terminal_model,
        session: Mutex::new(None),
        profile: Mutex::new(shell_profile(&profile)),
        profile_id: profile.id.clone(),
        profile_name: profile.name.clone(),
        event_tx,
        config: Mutex::new(config.clone()),
        config_path: loaded.path,
    });

    let frame = create_rw_signal(Arc::new(terminal.frame()));
    let font_size = create_rw_signal(config.terminal.font_size);
    let copy_on_select = create_rw_signal(config.clipboard.copy_on_select);
    let confirm_multiline = create_rw_signal(config.clipboard.confirm_multiline_paste);
    let locale = create_rw_signal(effective_locale(config.appearance.locale));
    let theme = create_rw_signal(config.appearance.theme);
    let status = create_rw_signal(SessionStatus::Starting);
    let title = create_rw_signal(profile.name.clone());
    let settings_open = create_rw_signal(false);
    let search_open = create_rw_signal(false);
    let search_query = create_rw_signal(String::new());
    let search_case_sensitive = create_rw_signal(false);
    let search_results = create_rw_signal(SearchResult::default());
    let preedit = create_rw_signal(String::new());
    let selecting = create_rw_signal(false);
    let terminal_mouse_button = create_rw_signal(TerminalMouseButton::None);
    let selection_start = create_rw_signal(CellPoint::default());
    let pending_paste = create_rw_signal(None::<String>);
    let pending_osc52 = create_rw_signal(None::<String>);
    let config_warning = create_rw_signal(loaded.warning);

    let event_signal = create_signal_from_channel(event_rx);
    {
        let state = Arc::clone(&state);
        create_effect(move |_| {
            let Some(event) = event_signal.get() else {
                return;
            };
            match event {
                SessionEvent::Output => status.set(SessionStatus::Running),
                SessionEvent::Exited { code, .. } => status.set(SessionStatus::Exited(code)),
                SessionEvent::ReadFailed(error) | SessionEvent::WriteFailed(error) => {
                    tracing::error!(%error, "terminal session I/O failed");
                    status.set(SessionStatus::Failed);
                }
            }
            frame.set(Arc::new(state.terminal.frame()));
            for action in state.terminal.drain_actions() {
                match action {
                    TerminalAction::Title(new_title) => title.set(new_title),
                    TerminalAction::ResetTitle => title.set(state.profile_name.clone()),
                    TerminalAction::Bell
                    | TerminalAction::Wakeup
                    | TerminalAction::WriteToHost(_) => {}
                    TerminalAction::ExitRequested => {
                        if let Some(session) = state.session.lock().as_mut() {
                            let _ = session.terminate();
                        }
                    }
                    TerminalAction::ClipboardStore { text, .. } => {
                        let trusted = state
                            .config
                            .lock()
                            .clipboard
                            .trusted_osc52_write_profiles
                            .contains(&state.profile_id);
                        if trusted {
                            let _ = Clipboard::set_contents(text);
                        } else {
                            pending_osc52.set(Some(text));
                        }
                    }
                    TerminalAction::ClipboardLoad { .. } => {
                        tracing::warn!("OSC 52 clipboard read denied by policy");
                    }
                }
            }
        });
    }

    {
        let terminal = Arc::clone(&terminal);
        create_effect(move |_| {
            let _revision = frame.get().revision;
            search_results.set(terminal.search(&SearchRequest {
                query: search_query.get(),
                case_sensitive: search_case_sensitive.get(),
            }));
        });
    }

    if let Err(error) = start_session(&state) {
        tracing::error!(%error, "failed to start initial shell");
        status.set(SessionStatus::Failed);
    }

    let terminal_surface = terminal_view(frame, font_size, search_results, preedit)
        .keyboard_navigable()
        .on_event_stop(EventListener::FocusGained, move |_| set_ime_allowed(true))
        .on_event_stop(EventListener::FocusLost, move |_| {
            set_ime_allowed(false);
            preedit.set(String::new());
        })
        .on_event_stop(EventListener::KeyDown, {
            let state = Arc::clone(&state);
            move |event| {
                if let Event::KeyDown(key) = event {
                    handle_key(
                        key,
                        &state,
                        frame,
                        font_size,
                        settings_open,
                        search_open,
                        pending_paste,
                        confirm_multiline,
                    );
                }
            }
        })
        .on_event_stop(EventListener::ImePreedit, move |event| {
            if let Event::ImePreedit { text, .. } = event {
                preedit.set(text.clone());
            }
        })
        .on_event_stop(EventListener::ImeCommit, {
            let state = Arc::clone(&state);
            move |event| {
                if let Event::ImeCommit(text) = event {
                    preedit.set(String::new());
                    write_session(&state, text.as_bytes().to_vec());
                }
            }
        })
        .on_event_stop(EventListener::PointerDown, {
            let terminal = Arc::clone(&terminal);
            let state = Arc::clone(&state);
            move |event| {
                if let Event::PointerDown(pointer) = event {
                    let current_frame = frame.get_untracked();
                    if current_frame.modes.mouse_tracking && !pointer.modifiers.shift() {
                        let button = terminal_mouse_button_from(pointer.button);
                        terminal_mouse_button.set(button);
                        let point =
                            point_to_cell(pointer.pos, font_size.get_untracked(), &current_frame);
                        write_session(
                            &state,
                            encode_mouse(
                                MouseInput {
                                    kind: MouseKind::Press,
                                    button,
                                    row: point.row,
                                    column: point.column,
                                    modifiers: terminal_modifiers(pointer.modifiers),
                                },
                                current_frame.modes,
                            ),
                        );
                        return;
                    }
                    let point =
                        point_to_cell(pointer.pos, font_size.get_untracked(), &current_frame);
                    let selection = match pointer.count {
                        3.. => CellRange {
                            start: CellPoint {
                                row: point.row,
                                column: 0,
                            },
                            end: CellPoint {
                                row: point.row,
                                column: current_frame.size.columns.saturating_sub(1),
                            },
                        },
                        2 => semantic_range(&current_frame, point),
                        _ => CellRange {
                            start: point,
                            end: point,
                        },
                    };
                    selection_start.set(selection.start);
                    selecting.set(pointer.count == 1);
                    terminal.set_selection(Some(selection));
                    frame.set(Arc::new(terminal.frame()));
                }
            }
        })
        .on_event_stop(EventListener::PointerMove, {
            let terminal = Arc::clone(&terminal);
            let state = Arc::clone(&state);
            move |event| {
                if selecting.get_untracked() {
                    if let Event::PointerMove(pointer) = event {
                        let current_frame = frame.get_untracked();
                        let end =
                            point_to_cell(pointer.pos, font_size.get_untracked(), &current_frame);
                        terminal.set_selection(Some(CellRange {
                            start: selection_start.get_untracked(),
                            end,
                        }));
                        frame.set(Arc::new(terminal.frame()));
                    }
                } else if let Event::PointerMove(pointer) = event {
                    let current_frame = frame.get_untracked();
                    if current_frame.modes.mouse_tracking && !pointer.modifiers.shift() {
                        let point =
                            point_to_cell(pointer.pos, font_size.get_untracked(), &current_frame);
                        write_session(
                            &state,
                            encode_mouse(
                                MouseInput {
                                    kind: MouseKind::Move,
                                    button: terminal_mouse_button.get_untracked(),
                                    row: point.row,
                                    column: point.column,
                                    modifiers: terminal_modifiers(pointer.modifiers),
                                },
                                current_frame.modes,
                            ),
                        );
                    }
                }
            }
        })
        .on_event_stop(EventListener::PointerUp, {
            let terminal = Arc::clone(&terminal);
            let state = Arc::clone(&state);
            move |event| {
                if let Event::PointerUp(pointer) = event {
                    let current_frame = frame.get_untracked();
                    if current_frame.modes.mouse_tracking && !pointer.modifiers.shift() {
                        let point =
                            point_to_cell(pointer.pos, font_size.get_untracked(), &current_frame);
                        write_session(
                            &state,
                            encode_mouse(
                                MouseInput {
                                    kind: MouseKind::Release,
                                    button: terminal_mouse_button.get_untracked(),
                                    row: point.row,
                                    column: point.column,
                                    modifiers: terminal_modifiers(pointer.modifiers),
                                },
                                current_frame.modes,
                            ),
                        );
                        terminal_mouse_button.set(TerminalMouseButton::None);
                        return;
                    }
                }
                selecting.set(false);
                if copy_on_select.get_untracked()
                    && let Some(text) = terminal.selection_text()
                {
                    let _ = Clipboard::set_contents(text);
                }
            }
        })
        .on_event_stop(EventListener::PointerWheel, {
            let terminal = Arc::clone(&terminal);
            let state = Arc::clone(&state);
            move |event| {
                if let Event::PointerWheel(pointer) = event {
                    let current_frame = frame.get_untracked();
                    if current_frame.modes.mouse_tracking && !pointer.modifiers.shift() {
                        let point =
                            point_to_cell(pointer.pos, font_size.get_untracked(), &current_frame);
                        let kind = if pointer.delta.y >= 0.0 {
                            MouseKind::WheelUp
                        } else {
                            MouseKind::WheelDown
                        };
                        write_session(
                            &state,
                            encode_mouse(
                                MouseInput {
                                    kind,
                                    button: TerminalMouseButton::None,
                                    row: point.row,
                                    column: point.column,
                                    modifiers: terminal_modifiers(pointer.modifiers),
                                },
                                current_frame.modes,
                            ),
                        );
                        return;
                    }
                    let lines =
                        (pointer.delta.y / line_height(font_size.get_untracked())).round() as i32;
                    if lines != 0 {
                        terminal.scroll(lines);
                        frame.set(Arc::new(terminal.frame()));
                    }
                }
            }
        })
        .on_resize({
            let state = Arc::clone(&state);
            move |rect| {
                let width = terminal_view::cell_width(font_size.get_untracked());
                let height = line_height(font_size.get_untracked());
                let columns = ((rect.width() - 16.0).max(width) / width).floor() as usize;
                let lines = ((rect.height() - 12.0).max(height) / height).floor() as usize;
                let size = TerminalSize::new(columns.max(2), lines.max(2));
                state.terminal.resize(size);
                if let Some(session) = state.session.lock().as_ref() {
                    let _ = session.resize(size);
                }
                frame.set(Arc::new(state.terminal.frame()));
            }
        })
        .style(|style| style.size_full().min_height(120.0));

    let chrome = v_stack((
        session_bar(Arc::clone(&state), title, status, settings_open, locale),
        terminal_surface,
        status_bar(frame, status, locale),
    ))
    .style(floem::style::Style::size_full);

    stack((
        chrome.into_any(),
        dyn_view(move || {
            if search_open.get() {
                search_overlay(
                    search_query,
                    search_case_sensitive,
                    search_results,
                    search_open,
                )
                .into_any()
            } else {
                empty().into_any()
            }
        })
        .into_any(),
        dyn_view({
            let state = Arc::clone(&state);
            move || {
                if settings_open.get() {
                    settings_overlay(
                        &state,
                        font_size,
                        copy_on_select,
                        confirm_multiline,
                        locale,
                        theme,
                        settings_open,
                    )
                    .into_any()
                } else {
                    empty().into_any()
                }
            }
        })
        .into_any(),
        dyn_view({
            let state = Arc::clone(&state);
            move || {
                if let Some(text) = pending_paste.get() {
                    confirmation_dialog(
                        tr(locale.get(), "paste_title"),
                        format!(
                            "{}\n\n{}",
                            tr(locale.get(), "paste_warning"),
                            preview(&text)
                        ),
                        tr(locale.get(), "paste"),
                        tr(locale.get(), "cancel"),
                        {
                            let state = Arc::clone(&state);
                            move || {
                                let modes = frame.get_untracked().modes;
                                write_session(&state, encode_paste(&text, modes));
                                pending_paste.set(None);
                            }
                        },
                        move || pending_paste.set(None),
                    )
                    .into_any()
                } else {
                    empty().into_any()
                }
            }
        })
        .into_any(),
        dyn_view({
            let state = Arc::clone(&state);
            move || {
                if let Some(text) = pending_osc52.get() {
                    osc52_dialog(&state, &text, pending_osc52, locale).into_any()
                } else {
                    empty().into_any()
                }
            }
        })
        .into_any(),
        dyn_view(move || {
            config_warning.get().map_or_else(
                || empty().into_any(),
                |warning| {
                    h_stack((
                        label(move || warning.clone()),
                        button(tr(locale.get(), "dismiss"))
                            .on_click_stop(move |_| config_warning.set(None)),
                    ))
                    .style(|style| {
                        style
                            .margin(12.0)
                            .padding(10.0)
                            .gap(10.0)
                            .background(Color::rgb8(105, 67, 36))
                    })
                    .into_any()
                },
            )
        })
        .into_any(),
    ))
    .style(move |style| {
        let background = match theme.get() {
            ThemePreference::Light => Color::rgb8(241, 245, 241),
            ThemePreference::MatchaDark | ThemePreference::System => Color::rgb8(24, 31, 26),
        };
        style.size_full().background(background)
    })
}

fn start_session(state: &Arc<WorkspaceState>) -> Result<(), matcha_pty::PtyError> {
    let session = LocalPtySession::spawn(
        &state.profile.lock(),
        state.terminal.size(),
        &state.terminal_model,
    )?;
    let events = session.events();
    let relay = state.event_tx.clone();
    thread::Builder::new()
        .name("matcha-ui-session-events".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                if relay.send(event).is_err() {
                    break;
                }
            }
        })
        .expect("session event relay thread must spawn");
    *state.session.lock() = Some(session);
    Ok(())
}

fn write_session(state: &WorkspaceState, bytes: Vec<u8>) {
    if let Some(session) = state.session.lock().as_ref()
        && let Err(error) = session.write(bytes)
    {
        tracing::error!(%error, "failed to write terminal input");
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_key(
    event: &floem::keyboard::KeyEvent,
    state: &Arc<WorkspaceState>,
    frame: RwSignal<Arc<TerminalFrame>>,
    font_size: RwSignal<f32>,
    settings_open: RwSignal<bool>,
    search_open: RwSignal<bool>,
    pending_paste: RwSignal<Option<String>>,
    confirm_multiline: RwSignal<bool>,
) {
    let control = event.modifiers.control();
    let shift = event.modifiers.shift();
    if control && shift {
        match &event.key.logical_key {
            Key::Character(character) if character.eq_ignore_ascii_case("c") => {
                if let Some(text) = state.terminal.selection_text() {
                    let _ = Clipboard::set_contents(text);
                }
                return;
            }
            Key::Character(character) if character.eq_ignore_ascii_case("v") => {
                if let Ok(text) = Clipboard::get_contents() {
                    if confirm_multiline.get_untracked() && contains_multiple_lines(&text) {
                        pending_paste.set(Some(text));
                    } else {
                        write_session(state, encode_paste(&text, frame.get_untracked().modes));
                    }
                }
                return;
            }
            _ => {}
        }
    }
    if control && let Key::Character(character) = &event.key.logical_key {
        match character.as_str() {
            "f" | "F" => {
                search_open.set(true);
                return;
            }
            "," => {
                settings_open.set(true);
                return;
            }
            "+" | "=" => {
                change_font_size(state, font_size, 1.0);
                return;
            }
            "-" => {
                change_font_size(state, font_size, -1.0);
                return;
            }
            "0" => {
                set_font_size(state, font_size, matcha_config::DEFAULT_FONT_SIZE);
                return;
            }
            _ => {}
        }
    }
    if let Some(input) = map_key(event) {
        write_session(state, encode_key(&input, frame.get_untracked().modes));
    }
}

fn map_key(event: &floem::keyboard::KeyEvent) -> Option<KeyInput> {
    let code = match &event.key.logical_key {
        Key::Character(character) => KeyCode::Character(character.to_string()),
        Key::Named(NamedKey::Enter) => KeyCode::Enter,
        Key::Named(NamedKey::Tab) => KeyCode::Tab,
        Key::Named(NamedKey::Backspace) => KeyCode::Backspace,
        Key::Named(NamedKey::Escape) => KeyCode::Escape,
        Key::Named(NamedKey::ArrowUp) => KeyCode::Up,
        Key::Named(NamedKey::ArrowDown) => KeyCode::Down,
        Key::Named(NamedKey::ArrowRight) => KeyCode::Right,
        Key::Named(NamedKey::ArrowLeft) => KeyCode::Left,
        Key::Named(NamedKey::Home) => KeyCode::Home,
        Key::Named(NamedKey::End) => KeyCode::End,
        Key::Named(NamedKey::Insert) => KeyCode::Insert,
        Key::Named(NamedKey::Delete) => KeyCode::Delete,
        Key::Named(NamedKey::PageUp) => KeyCode::PageUp,
        Key::Named(NamedKey::PageDown) => KeyCode::PageDown,
        Key::Named(named) => function_number(*named).map(KeyCode::Function)?,
        _ => return None,
    };
    Some(KeyInput {
        code,
        modifiers: Modifiers {
            shift: event.modifiers.shift(),
            alt: event.modifiers.alt(),
            control: event.modifiers.control(),
        },
    })
}

fn terminal_modifiers(modifiers: floem::keyboard::Modifiers) -> Modifiers {
    Modifiers {
        shift: modifiers.shift(),
        alt: modifiers.alt(),
        control: modifiers.control(),
    }
}

fn terminal_mouse_button_from(button: floem::pointer::PointerButton) -> TerminalMouseButton {
    if button.is_primary() {
        TerminalMouseButton::Left
    } else if button.is_auxiliary() {
        TerminalMouseButton::Middle
    } else if button.is_secondary() {
        TerminalMouseButton::Right
    } else {
        TerminalMouseButton::None
    }
}

fn function_number(key: NamedKey) -> Option<u8> {
    Some(match key {
        NamedKey::F1 => 1,
        NamedKey::F2 => 2,
        NamedKey::F3 => 3,
        NamedKey::F4 => 4,
        NamedKey::F5 => 5,
        NamedKey::F6 => 6,
        NamedKey::F7 => 7,
        NamedKey::F8 => 8,
        NamedKey::F9 => 9,
        NamedKey::F10 => 10,
        NamedKey::F11 => 11,
        NamedKey::F12 => 12,
        _ => return None,
    })
}

#[allow(clippy::needless_pass_by_value)]
fn session_bar(
    state: Arc<WorkspaceState>,
    title: RwSignal<String>,
    status: RwSignal<SessionStatus>,
    settings_open: RwSignal<bool>,
    locale: RwSignal<LocalePreference>,
) -> impl IntoView {
    let state_restart = Arc::clone(&state);
    h_stack((
        label(|| "Matcha").style(|style| style.font_size(18.0).font_bold()),
        label({
            let profile = state.profile_name.clone();
            move || format!("{profile} · {}", title.get())
        })
        .style(|style| style.flex_grow(1.0)),
        dyn_view(move || match status.get() {
            SessionStatus::Exited(_) | SessionStatus::Failed => button(tr(locale.get(), "restart"))
                .on_click_stop({
                    let state = Arc::clone(&state_restart);
                    move |_| {
                        status.set(SessionStatus::Starting);
                        if start_session(&state).is_err() {
                            status.set(SessionStatus::Failed);
                        }
                    }
                })
                .into_any(),
            SessionStatus::Starting | SessionStatus::Running => empty().into_any(),
        }),
        button("⚙").on_click_stop(move |_| settings_open.set(true)),
    ))
    .style(|style| {
        style
            .width_full()
            .height(44.0)
            .items_center()
            .gap(12.0)
            .padding_horiz(14.0)
            .color(Color::rgb8(226, 235, 226))
            .background(Color::rgb8(31, 42, 34))
    })
}

fn status_bar(
    frame: RwSignal<Arc<TerminalFrame>>,
    status: RwSignal<SessionStatus>,
    locale: RwSignal<LocalePreference>,
) -> impl IntoView {
    h_stack((
        label(move || match status.get() {
            SessionStatus::Starting => tr(locale.get(), "starting"),
            SessionStatus::Running => tr(locale.get(), "running"),
            SessionStatus::Exited(code) => format!("{} {code}", tr(locale.get(), "exited")),
            SessionStatus::Failed => tr(locale.get(), "failed"),
        }),
        label(move || {
            let size = frame.get().size;
            format!("{} × {}", size.columns, size.lines)
        })
        .style(|style| style.flex_grow(1.0)),
    ))
    .style(|style| {
        style
            .width_full()
            .height(28.0)
            .items_center()
            .padding_horiz(10.0)
            .font_size(12.0)
            .color(Color::rgb8(176, 193, 179))
            .background(Color::rgb8(27, 36, 30))
    })
}

fn search_overlay(
    query: RwSignal<String>,
    case_sensitive: RwSignal<bool>,
    results: RwSignal<SearchResult>,
    open: RwSignal<bool>,
) -> impl IntoView {
    h_stack((
        text_input(query).style(|style| style.width(280.0)),
        button("Aa").on_click_stop(move |_| case_sensitive.update(|value| *value = !*value)),
        label(move || {
            let result = results.get();
            if result.matches.is_empty() {
                "0 / 0".into()
            } else {
                format!(
                    "{} / {}",
                    result.active.unwrap_or(0) + 1,
                    result.matches.len()
                )
            }
        }),
        button("↑").on_click_stop(move |_| move_search(results, -1)),
        button("↓").on_click_stop(move |_| move_search(results, 1)),
        button("×").on_click_stop(move |_| open.set(false)),
    ))
    .on_event_stop(EventListener::KeyDown, move |event| {
        if let Event::KeyDown(key) = event {
            match key.key.logical_key {
                Key::Named(NamedKey::Enter) => {
                    move_search(results, if key.modifiers.shift() { -1 } else { 1 });
                }
                Key::Named(NamedKey::Escape) => open.set(false),
                _ => {}
            }
        }
    })
    .style(|style| {
        style
            .absolute()
            .margin_top(52.0)
            .margin_right(16.0)
            .inset_right(0.0)
            .padding(8.0)
            .gap(6.0)
            .items_center()
            .background(Color::rgb8(45, 58, 49))
    })
}

#[allow(clippy::too_many_arguments)]
fn settings_overlay(
    state: &Arc<WorkspaceState>,
    font_size: RwSignal<f32>,
    copy_on_select: RwSignal<bool>,
    confirm_multiline: RwSignal<bool>,
    locale: RwSignal<LocalePreference>,
    theme: RwSignal<ThemePreference>,
    open: RwSignal<bool>,
) -> impl IntoView {
    let profile = state.config.lock().default_shell().cloned();
    let state_font_down = Arc::clone(state);
    let state_font_up = Arc::clone(state);
    let state_copy = Arc::clone(state);
    let state_paste = Arc::clone(state);
    let state_locale_zh = Arc::clone(state);
    let state_locale_en = Arc::clone(state);
    let state_theme_dark = Arc::clone(state);
    let state_theme_light = Arc::clone(state);
    v_stack((
        h_stack((
            label(move || tr(locale.get(), "settings"))
                .style(|style| style.font_size(22.0).font_bold()),
            button("×")
                .on_click_stop(move |_| open.set(false))
                .style(|style| style.flex_grow(1.0)),
        )),
        label(move || tr(locale.get(), "terminal")).style(floem::style::Style::font_bold),
        h_stack((
            label(move || {
                format!(
                    "{}: {:.0}px",
                    tr(locale.get(), "font_size"),
                    font_size.get()
                )
            }),
            button("−").on_click_stop(move |_| change_font_size(&state_font_down, font_size, -1.0)),
            button("+").on_click_stop(move |_| change_font_size(&state_font_up, font_size, 1.0)),
        ))
        .style(|style| style.gap(8.0).items_center()),
        button(label(move || {
            toggle_label(locale.get(), "copy_on_select", copy_on_select.get())
        }))
        .on_click_stop(move |_| {
            copy_on_select.update(|value| *value = !*value);
            update_config(&state_copy, |config| {
                config.clipboard.copy_on_select = copy_on_select.get_untracked();
            });
        }),
        button(label(move || {
            toggle_label(locale.get(), "confirm_paste", confirm_multiline.get())
        }))
        .on_click_stop(move |_| {
            confirm_multiline.update(|value| *value = !*value);
            update_config(&state_paste, |config| {
                config.clipboard.confirm_multiline_paste = confirm_multiline.get_untracked();
            });
        }),
        label(move || tr(locale.get(), "appearance"))
            .style(|style| style.font_bold().margin_top(10.0)),
        h_stack((
            button("简体中文").on_click_stop(move |_| {
                locale.set(LocalePreference::SimplifiedChinese);
                update_config(&state_locale_zh, |config| {
                    config.appearance.locale = LocalePreference::SimplifiedChinese;
                });
            }),
            button("English").on_click_stop(move |_| {
                locale.set(LocalePreference::English);
                update_config(&state_locale_en, |config| {
                    config.appearance.locale = LocalePreference::English;
                });
            }),
        ))
        .style(|style| style.gap(8.0)),
        h_stack((
            button("Matcha Dark").on_click_stop(move |_| {
                theme.set(ThemePreference::MatchaDark);
                update_config(&state_theme_dark, |config| {
                    config.appearance.theme = ThemePreference::MatchaDark;
                });
            }),
            button("Light").on_click_stop(move |_| {
                theme.set(ThemePreference::Light);
                update_config(&state_theme_light, |config| {
                    config.appearance.theme = ThemePreference::Light;
                });
            }),
        ))
        .style(|style| style.gap(8.0)),
        label(move || tr(locale.get(), "shell")).style(|style| style.font_bold().margin_top(10.0)),
        label(move || {
            profile.as_ref().map_or_else(
                || tr(locale.get(), "no_shell"),
                |profile| {
                    format!(
                        "{}\n{} {}",
                        profile.name,
                        profile.program.display(),
                        profile.args.join(" ")
                    )
                },
            )
        }),
    ))
    .style(|style| {
        style
            .absolute()
            .inset(0.0)
            .padding(28.0)
            .gap(12.0)
            .color(Color::rgb8(228, 237, 228))
            .background(Color::rgba8(28, 38, 31, 250))
    })
}

fn confirmation_dialog(
    title: String,
    message: String,
    accept: String,
    cancel: String,
    on_accept: impl Fn() + 'static,
    on_cancel: impl Fn() + 'static,
) -> impl IntoView {
    v_stack((
        label(move || title.clone()).style(|style| style.font_size(18.0).font_bold()),
        label(move || message.clone()),
        h_stack((
            button(accept).on_click_stop(move |_| on_accept()),
            button(cancel).on_click_stop(move |_| on_cancel()),
        ))
        .style(|style| style.gap(8.0)),
    ))
    .style(|style| {
        style
            .absolute()
            .margin(80.0)
            .padding(20.0)
            .gap(12.0)
            .color(Color::WHITE)
            .background(Color::rgb8(48, 62, 52))
    })
}

fn osc52_dialog(
    state: &Arc<WorkspaceState>,
    text: &str,
    pending: RwSignal<Option<String>>,
    locale: RwSignal<LocalePreference>,
) -> impl IntoView {
    let once_text = text.to_owned();
    let always_text = text.to_owned();
    let state_always = Arc::clone(state);
    let title = tr(locale.get(), "clipboard_request");
    let message = format!("{}: {} bytes", state.profile_name, text.len());
    v_stack((
        label(move || title.clone()).style(|style| style.font_size(18.0).font_bold()),
        label(move || message.clone()),
        h_stack((
            button(tr(locale.get(), "allow_once")).on_click_stop(move |_| {
                let _ = Clipboard::set_contents(once_text.clone());
                pending.set(None);
            }),
            button(tr(locale.get(), "always_allow")).on_click_stop(move |_| {
                let _ = Clipboard::set_contents(always_text.clone());
                update_config(&state_always, |config| {
                    if !config
                        .clipboard
                        .trusted_osc52_write_profiles
                        .contains(&state_always.profile_id)
                    {
                        config
                            .clipboard
                            .trusted_osc52_write_profiles
                            .push(state_always.profile_id.clone());
                    }
                });
                pending.set(None);
            }),
            button(tr(locale.get(), "deny")).on_click_stop(move |_| pending.set(None)),
        ))
        .style(|style| style.gap(8.0)),
    ))
    .style(|style| {
        style
            .absolute()
            .margin(80.0)
            .padding(20.0)
            .gap(12.0)
            .color(Color::WHITE)
            .background(Color::rgb8(48, 62, 52))
    })
}

fn move_search(results: RwSignal<SearchResult>, delta: isize) {
    results.update(|result| {
        if result.matches.is_empty() {
            result.active = None;
            return;
        }
        let current = result.active.unwrap_or(0).cast_signed();
        let len = result.matches.len().cast_signed();
        result.active = Some((current + delta).rem_euclid(len) as usize);
    });
}

fn semantic_range(frame: &TerminalFrame, point: CellPoint) -> CellRange {
    let mut start = point.column;
    let mut end = point.column;
    let is_word = |column: usize| {
        frame
            .cells
            .iter()
            .find(|cell| cell.row == point.row && cell.column == column)
            .is_some_and(|cell| !cell.text.chars().all(char::is_whitespace))
    };
    while start > 0 && is_word(start - 1) {
        start -= 1;
    }
    while end + 1 < frame.size.columns && is_word(end + 1) {
        end += 1;
    }
    CellRange {
        start: CellPoint {
            row: point.row,
            column: start,
        },
        end: CellPoint {
            row: point.row,
            column: end,
        },
    }
}

fn update_config(state: &WorkspaceState, update: impl FnOnce(&mut AppConfig)) {
    let mut config = state.config.lock();
    update(&mut config);
    if let Err(error) = matcha_config::save(&state.config_path, &config) {
        tracing::error!(%error, "failed to save configuration");
    }
}

fn change_font_size(state: &WorkspaceState, signal: RwSignal<f32>, delta: f32) {
    set_font_size(
        state,
        signal,
        (signal.get_untracked() + delta).clamp(8.0, 48.0),
    );
}

fn set_font_size(state: &WorkspaceState, signal: RwSignal<f32>, value: f32) {
    signal.set(value);
    update_config(state, |config| config.terminal.font_size = value);
}

fn shell_profile(profile: &ShellProfileConfig) -> ShellProfile {
    ShellProfile {
        program: profile.program.clone(),
        args: profile.args.clone(),
        cwd: profile.startup_directory.clone(),
    }
}

fn fallback_profile() -> ShellProfileConfig {
    matcha_config::discover_shell_profiles()
        .into_iter()
        .next()
        .unwrap_or_else(|| ShellProfileConfig {
            id: "fallback".into(),
            name: "Shell".into(),
            program: if cfg!(windows) {
                "cmd.exe".into()
            } else {
                "/bin/sh".into()
            },
            args: Vec::new(),
            startup_directory: None,
        })
}

fn effective_locale(preference: LocalePreference) -> LocalePreference {
    if preference != LocalePreference::System {
        return preference;
    }
    sys_locale::get_locale().map_or(LocalePreference::English, |locale| {
        if locale.to_ascii_lowercase().starts_with("zh") {
            LocalePreference::SimplifiedChinese
        } else {
            LocalePreference::English
        }
    })
}

fn tr(locale: LocalePreference, key: &str) -> String {
    let chinese = locale == LocalePreference::SimplifiedChinese;
    match (chinese, key) {
        (true, "restart") => "重新启动",
        (true, "starting") => "正在启动",
        (true, "running") => "运行中",
        (true, "exited") => "已退出，代码",
        (true, "failed") => "会话失败",
        (true, "settings") => "设置",
        (true, "terminal") => "终端",
        (true, "font_size") => "字号",
        (true, "copy_on_select") => "选中即复制",
        (true, "confirm_paste") => "多行粘贴前确认",
        (true, "appearance") => "外观与语言",
        (true, "shell") => "默认 Shell Profile",
        (true, "no_shell") => "没有可用的 Shell",
        (true, "paste_title") => "确认多行粘贴",
        (true, "paste_warning") => "以下内容包含多行，可能执行多条命令：",
        (true, "paste") => "粘贴",
        (true, "cancel") => "取消",
        (true, "clipboard_request") => "终端请求写入剪贴板",
        (true, "allow_once") => "允许一次",
        (true, "always_allow") => "始终允许此配置",
        (true, "deny") => "拒绝",
        (true, "dismiss") => "关闭",
        (false, "restart") => "Restart",
        (false, "starting") => "Starting",
        (false, "running") => "Running",
        (false, "exited") => "Exited with code",
        (false, "failed") => "Session failed",
        (false, "settings") => "Settings",
        (false, "terminal") => "Terminal",
        (false, "font_size") => "Font size",
        (false, "copy_on_select") => "Copy on select",
        (false, "confirm_paste" | "paste_title") => "Confirm multiline paste",
        (false, "appearance") => "Appearance and language",
        (false, "shell") => "Default shell profile",
        (false, "no_shell") => "No shell is available",
        (false, "paste_warning") => {
            "This content contains multiple lines and may run multiple commands:"
        }
        (false, "paste") => "Paste",
        (false, "cancel") => "Cancel",
        (false, "clipboard_request") => "Terminal clipboard write request",
        (false, "allow_once") => "Allow once",
        (false, "always_allow") => "Always allow this profile",
        (false, "deny") => "Deny",
        (false, "dismiss") => "Dismiss",
        _ => key,
    }
    .into()
}

fn toggle_label(locale: LocalePreference, key: &str, enabled: bool) -> String {
    format!("{}: {}", tr(locale, key), if enabled { "✓" } else { "○" })
}

fn contains_multiple_lines(text: &str) -> bool {
    text.contains('\n') || text.contains('\r')
}

fn preview(text: &str) -> String {
    const LIMIT: usize = 600;
    let mut preview: String = text.chars().take(LIMIT).collect();
    if text.chars().count() > LIMIT {
        preview.push_str("\n…");
    }
    preview
}
