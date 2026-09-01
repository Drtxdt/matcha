//! Local PTY lifecycle and blocking-I/O isolation.

use std::io::{Read, Write};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use matcha_core::ShellProfile;
use matcha_terminal::{TerminalModel, TerminalSize};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("failed to open local PTY: {0}")]
    Open(#[source] anyhow::Error),
    #[error("failed to clone PTY reader: {0}")]
    CloneReader(#[source] anyhow::Error),
    #[error("failed to acquire PTY writer: {0}")]
    Writer(#[source] anyhow::Error),
    #[error("failed to spawn shell: {0}")]
    Spawn(#[source] anyhow::Error),
    #[error("failed to spawn {name} thread: {source}")]
    SpawnThread {
        name: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("terminal input channel is closed")]
    InputClosed,
    #[error("terminal input queue is full")]
    InputBackpressure,
    #[error("failed to resize PTY: {0}")]
    Resize(#[source] anyhow::Error),
    #[error("failed to terminate shell: {0}")]
    Terminate(#[source] std::io::Error),
}

/// Events emitted by a local terminal session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionEvent {
    Output,
    Exited { code: u32, signal: Option<String> },
    ReadFailed(String),
    WriteFailed(String),
}

/// Owns a local PTY and forwards bytes into a terminal model.
pub struct LocalPtySession {
    master: Box<dyn MasterPty + Send>,
    child_killer: Box<dyn ChildKiller + Send + Sync>,
    input: Sender<Vec<u8>>,
    events: Receiver<SessionEvent>,
    child_thread: Option<JoinHandle<()>>,
    reader_thread: Option<JoinHandle<()>>,
    writer_thread: Option<JoinHandle<()>>,
}

impl LocalPtySession {
    /// Starts a local shell connected to `terminal` through a native PTY.
    ///
    /// # Errors
    ///
    /// Returns an error when the PTY, shell, I/O handles, or worker threads
    /// cannot be created.
    pub fn spawn(
        shell: &ShellProfile,
        size: TerminalSize,
        terminal: &Arc<dyn TerminalModel>,
    ) -> Result<Self, PtyError> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(to_pty_size(size))
            .map_err(PtyError::Open)?;

        let mut command = CommandBuilder::new(&shell.program);
        command.args(&shell.args);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        if let Some(cwd) = &shell.cwd {
            command.cwd(cwd);
        }
        let mut child = pair.slave.spawn_command(command).map_err(PtyError::Spawn)?;
        let child_killer = child.clone_killer();
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(PtyError::CloneReader)?;
        let mut writer = pair.master.take_writer().map_err(PtyError::Writer)?;
        let (input_tx, input_rx) = bounded::<Vec<u8>>(256);
        let (event_tx, event_rx) = bounded::<SessionEvent>(2);

        // Waiting is required to reliably drain Windows ConPTY output. Keep it
        // off the UI thread and retain a separate killer handle for shutdown.
        let child_events = event_tx.clone();
        let child_thread = thread::Builder::new()
            .name("matcha-pty-child".into())
            .spawn(move || {
                let event = match child.wait() {
                    Ok(status) => SessionEvent::Exited {
                        code: status.exit_code(),
                        signal: status.signal().map(str::to_owned),
                    },
                    Err(error) => SessionEvent::ReadFailed(error.to_string()),
                };
                let _ = child_events.send(event);
            })
            .map_err(|source| PtyError::SpawnThread {
                name: "PTY child",
                source,
            })?;

        let reader_events = event_tx.clone();
        let reader_terminal = Arc::clone(terminal);
        let host_response_input = input_tx.clone();
        let reader_thread = thread::Builder::new()
            .name("matcha-pty-reader".into())
            .spawn(move || {
                read_loop(
                    &mut reader,
                    &*reader_terminal,
                    &host_response_input,
                    &reader_events,
                );
            })
            .map_err(|source| PtyError::SpawnThread {
                name: "PTY reader",
                source,
            })?;

        let writer_thread = thread::Builder::new()
            .name("matcha-pty-writer".into())
            .spawn(move || {
                while let Ok(first) = input_rx.recv() {
                    let mut bytes = first;
                    while bytes.len() < 64 * 1024 {
                        let Ok(next) = input_rx.try_recv() else {
                            break;
                        };
                        bytes.extend_from_slice(&next);
                    }
                    if let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
                        let _ = event_tx.send(SessionEvent::WriteFailed(error.to_string()));
                        return;
                    }
                }
            })
            .map_err(|source| PtyError::SpawnThread {
                name: "PTY writer",
                source,
            })?;

        Ok(Self {
            master: pair.master,
            child_killer,
            input: input_tx,
            events: event_rx,
            child_thread: Some(child_thread),
            reader_thread: Some(reader_thread),
            writer_thread: Some(writer_thread),
        })
    }

    /// Queues user or terminal-generated bytes for the shell.
    ///
    /// # Errors
    ///
    /// Returns [`PtyError::InputClosed`] after the writer worker has stopped.
    pub fn write(&self, bytes: impl Into<Vec<u8>>) -> Result<(), PtyError> {
        self.input
            .try_send(bytes.into())
            .map_err(|error| match error {
                TrySendError::Full(_) => PtyError::InputBackpressure,
                TrySendError::Disconnected(_) => PtyError::InputClosed,
            })
    }

    #[must_use]
    pub fn events(&self) -> Receiver<SessionEvent> {
        self.events.clone()
    }

    /// Resizes the native PTY, clamping dimensions to its 16-bit protocol.
    ///
    /// # Errors
    ///
    /// Returns an error when the platform PTY rejects the resize request.
    pub fn resize(&self, size: TerminalSize) -> Result<(), PtyError> {
        self.master
            .resize(to_pty_size(size))
            .map_err(PtyError::Resize)
    }

    /// Requests termination without waiting on the UI thread.
    ///
    /// # Errors
    ///
    /// Returns an error when the platform cannot signal the child process.
    pub fn terminate(&mut self) -> Result<(), PtyError> {
        self.child_killer.kill().map_err(PtyError::Terminate)
    }
}

impl Drop for LocalPtySession {
    fn drop(&mut self) {
        let _ = self.child_killer.kill();
        let handles = [
            self.child_thread.take(),
            self.reader_thread.take(),
            self.writer_thread.take(),
        ];
        // Joining ConPTY workers can block while Windows drains its final output.
        // Reap them outside the UI thread after this session's channels are dropped.
        let _ = thread::Builder::new()
            .name("matcha-pty-reaper".into())
            .spawn(move || {
                for handle in handles.into_iter().flatten() {
                    let _ = handle.join();
                }
            });
    }
}

fn read_loop(
    reader: &mut dyn Read,
    terminal: &dyn TerminalModel,
    input: &Sender<Vec<u8>>,
    events: &Sender<SessionEvent>,
) {
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                return;
            }
            Ok(read) => {
                terminal.feed(&buffer[..read]);
                for response in terminal.drain_host_responses() {
                    let _ = input.send(response);
                }
                let _ = events.try_send(SessionEvent::Output);
            }
            Err(error) => {
                let _ = events.send(SessionEvent::ReadFailed(error.to_string()));
                return;
            }
        }
    }
}

fn to_pty_size(size: TerminalSize) -> PtySize {
    PtySize {
        rows: u16::try_from(size.lines).unwrap_or(u16::MAX),
        cols: u16::try_from(size.columns).unwrap_or(u16::MAX),
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use matcha_terminal::AlacrittyTerminal;

    use super::*;

    #[test]
    fn local_shell_output_reaches_terminal_model() {
        exercise_shell(&marker_shell());
    }

    #[test]
    fn terminal_environment_enables_modern_color_prompts() {
        let shell = marker_shell();
        let terminal = Arc::new(AlacrittyTerminal::new(TerminalSize::new(80, 12)));
        let terminal_model: Arc<dyn TerminalModel> = terminal.clone();
        let session = LocalPtySession::spawn(&shell, terminal.size(), &terminal_model)
            .expect("local PTY should start");
        session
            .write(color_environment_command())
            .expect("environment command should queue");
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let visible = terminal.visible_text();
            if visible.contains("xterm-256color") && visible.contains("truecolor") {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "terminal color environment was not visible: {:?}",
            terminal.visible_text()
        );
    }

    #[cfg(windows)]
    #[test]
    fn every_discovered_windows_shell_runs_and_exits_cleanly() {
        let profiles = matcha_config::discover_shell_profiles();
        assert!(
            profiles
                .iter()
                .any(|profile| profile.id == "windows-powershell")
        );
        assert!(
            profiles
                .iter()
                .any(|profile| profile.id == "command-prompt")
        );
        for profile in profiles {
            exercise_shell(&ShellProfile {
                program: profile.program,
                args: profile.args,
                cwd: profile.startup_directory,
            });
        }
    }

    fn exercise_shell(shell: &ShellProfile) {
        let terminal = Arc::new(AlacrittyTerminal::new(TerminalSize::new(80, 12)));
        let terminal_model: Arc<dyn TerminalModel> = terminal.clone();
        let session = LocalPtySession::spawn(shell, terminal.size(), &terminal_model)
            .expect("local PTY should start");
        for byte in marker_input() {
            session
                .write(vec![*byte])
                .expect("marker command byte should be queued");
        }
        let events = session.events();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut observed = Vec::new();
        let mut marker_seen = false;
        let mut clean_exit = false;

        while Instant::now() < deadline {
            marker_seen |= terminal.visible_text().contains("MATCHA_PTY_OK  SPACE_OK");
            clean_exit |= observed.iter().any(|event| {
                matches!(
                    event,
                    SessionEvent::Exited {
                        code: 0,
                        signal: None
                    }
                )
            });
            if marker_seen && clean_exit {
                return;
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            match events.recv_timeout(remaining.min(Duration::from_millis(250))) {
                Ok(event) => observed.push(event),
                Err(_) if Instant::now() >= deadline => break,
                Err(_) => {}
            }
        }

        panic!(
            "PTY shell {:?} did not produce its marker and clean exit; events: {observed:?}; visible output: {:?}",
            shell.program,
            terminal.visible_text(),
        );
    }

    #[test]
    #[ignore = "release-only local shell echo latency acceptance"]
    fn local_shell_echo_p95_is_below_fifty_milliseconds() {
        let terminal = Arc::new(AlacrittyTerminal::new(TerminalSize::new(100, 20)));
        let terminal_model: Arc<dyn TerminalModel> = terminal.clone();
        let session = LocalPtySession::spawn(&marker_shell(), terminal.size(), &terminal_model)
            .expect("local PTY should start");
        let mut samples = Vec::with_capacity(100);
        for index in 0..105 {
            let marker = format!("MATCHA_LATENCY_{index}");
            let command = latency_command(&marker);
            let started = Instant::now();
            session
                .write(command)
                .expect("latency command should queue");
            let deadline = started + Duration::from_secs(2);
            while !terminal.visible_text().contains(&marker) {
                assert!(Instant::now() < deadline, "shell did not echo {marker}");
                std::thread::sleep(Duration::from_millis(1));
            }
            if index >= 5 {
                samples.push(started.elapsed());
            }
        }
        samples.sort_unstable();
        let p95 = samples[94];
        eprintln!("local shell echo p95: {p95:?}");
        assert!(
            p95 <= Duration::from_millis(50),
            "local shell echo p95 exceeded 50 ms: {p95:?}"
        );
    }

    #[cfg(windows)]
    fn marker_shell() -> ShellProfile {
        ShellProfile {
            program: PathBuf::from("cmd.exe"),
            args: vec!["/D".into()],
            cwd: None,
        }
    }

    #[cfg(windows)]
    fn marker_input() -> &'static [u8] {
        b"echo MATCHA_PTY_OK  SPACE_OK\r\nexit\r\n"
    }

    #[cfg(windows)]
    fn latency_command(marker: &str) -> Vec<u8> {
        format!("echo {marker}\r\n").into_bytes()
    }

    #[cfg(windows)]
    fn color_environment_command() -> &'static [u8] {
        b"echo %TERM% %COLORTERM%\r\n"
    }

    #[cfg(not(windows))]
    fn marker_shell() -> ShellProfile {
        ShellProfile {
            program: PathBuf::from("/bin/sh"),
            args: Vec::new(),
            cwd: None,
        }
    }

    #[cfg(not(windows))]
    fn marker_input() -> &'static [u8] {
        b"printf 'MATCHA_PTY_OK  SPACE_OK\\n'\nexit\n"
    }

    #[cfg(not(windows))]
    fn latency_command(marker: &str) -> Vec<u8> {
        format!("printf '{marker}\\n'\n").into_bytes()
    }

    #[cfg(not(windows))]
    fn color_environment_command() -> &'static [u8] {
        b"printf '%s %s\\n' \"$TERM\" \"$COLORTERM\"\n"
    }
}
