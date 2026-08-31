# M0 validation checklist

- [x] Rust workspace and dependency boundaries exist.
- [x] Alacritty is isolated behind a Matcha-owned trait.
- [x] Local PTY blocking I/O is isolated from the UI thread.
- [x] Floem is isolated in the UI crate.
- [x] A live terminal grid renders in Floem and the Windows launch smoke test opens a shell.
- [x] Keyboard, IME events, selection, scrollback-wide search, resize and guarded clipboard flows are connected.
- [x] Configuration round-trip, malformed recovery, shell output, ANSI parsing and input encoding have automated tests.
- [x] Format, strict Clippy and workspace tests pass locally.
- [x] Incremental Alacritty damage and scrollback-wide search drive the renderer row cache.
- [x] Cursor blinking, font/scrollback/profile editing controls and live system-theme selection are complete.
- [x] The 100,000-line release performance scenario and idle-CPU measurement pass locally on Windows 11 (0.84 s backend stress case; 0.0163% normalized CPU over a 15 s idle sample).
- [ ] Windows 10/11 and Ubuntu 22.04/24.04 smoke tests pass.
- [ ] PowerShell 5.1/7, bash, zsh, tmux and Neovim compatibility checks pass.

Unchecked items are release gates. In particular, Ubuntu GUI/font/Chinese IME checks must remain unchecked until performed on real Ubuntu 22.04 and 24.04 systems.

The automated 100,000-line case is intentionally ignored in ordinary debug test runs. Run it with `cargo test -p matcha-terminal --release retains_configured_history_after_one_hundred_thousand_lines -- --ignored` and record elapsed time plus an idle-CPU observation before checking the performance gate.
