# M0 validation checklist

- [x] Rust workspace and dependency boundaries exist.
- [x] Alacritty is isolated behind a Matcha-owned trait.
- [x] Local PTY blocking I/O is isolated from the UI thread.
- [x] Floem is isolated in the UI crate.
- [x] A live terminal grid renders in Floem and the Windows launch smoke test opens a shell.
- [x] Keyboard, IME events, selection, visible-grid search, resize and guarded clipboard flows are connected.
- [x] Configuration round-trip, malformed recovery, shell output, ANSI parsing and input encoding have automated tests.
- [x] Format, strict Clippy and workspace tests pass locally.
- [ ] Incremental Alacritty damage and scrollback-wide search replace the current full-frame/visible-grid implementation.
- [ ] Cursor blinking, font/scrollback/profile editing controls and full system-theme selection are complete.
- [ ] The 100,000-line release performance scenario and idle-CPU measurement pass.
- [ ] Windows 10/11 and Ubuntu 22.04/24.04 smoke tests pass.
- [ ] PowerShell 5.1/7, bash, zsh, tmux and Neovim compatibility checks pass.

Unchecked items are release gates. In particular, Ubuntu GUI/font/Chinese IME checks must remain unchecked until performed on real Ubuntu 22.04 and 24.04 systems.
