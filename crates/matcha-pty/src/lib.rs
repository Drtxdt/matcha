//! Local PTY lifecycle and blocking-I/O isolation.

use std::io::{Read, Write};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender, bounded};
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
    _child_thread: JoinHandle<()>,
    _reader_thread: JoinHandle<()>,
    _writer_thread: JoinHandle<()>,
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
        let (input_tx, input_rx) = bounded::<Vec<u8>>(128);
        let (event_tx, event_rx) = bounded::<SessionEvent>(64);

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
                while let Ok(bytes) = input_rx.recv() {
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
            _child_thread: child_thread,
            _reader_thread: reader_thread,
            _writer_thread: writer_thread,
        })
    }

    /// Queues user or terminal-generated bytes for the shell.
    ///
    /// # Errors
    ///
    /// Returns [`PtyError::InputClosed`] after the writer worker has stopped.
    pub fn write(&self, bytes: impl Into<Vec<u8>>) -> Result<(), PtyError> {
        self.input
            .send(bytes.into())
            .map_err(|_| PtyError::InputClosed)
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
        let shell = marker_shell();
        let terminal = Arc::new(AlacrittyTerminal::new(TerminalSize::new(80, 12)));
        let terminal_model: Arc<dyn TerminalModel> = terminal.clone();
        let session = LocalPtySession::spawn(&shell, terminal.size(), &terminal_model)
            .expect("local PTY should start");
        session
            .write(marker_input())
            .expect("marker command should be written");
        let events = session.events();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut observed = Vec::new();

        while Instant::now() < deadline {
            if terminal.visible_text().contains("MATCHA_PTY_OK") {
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
            "PTY marker did not reach terminal; events: {observed:?}; visible output: {:?}",
            terminal.visible_text(),
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
        b"echo MATCHA_PTY_OK\r\nexit\r\n"
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
        b"printf MATCHA_PTY_OK\nexit\n"
    }
}
