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
- [x] Configuration schema v2 selects the bundled `JetBrains Mono NL` by its real family name and provides a clamped 1.0–1.5 line-height setting (default 1.15).
- [x] Space and modified-space input are encoded, PTY writes are opportunistically batched, and output wakeups are coalesced without losing the final frame.
- [x] Background, selection and search spans share DPI-snapped physical-pixel boundaries without per-cell antialiasing seams.
- [x] Local PTYs advertise `TERM=xterm-256color` and `COLORTERM=truecolor` so Starship and other color-aware prompts do not fall back to dumb-terminal behavior.
- [x] The 100,000-line release performance scenario and idle-CPU measurement pass locally on Windows 11 (0.84 s backend stress case; 0.0163% normalized CPU over a 15 s idle sample).
- [ ] Windows 10/11 and Ubuntu 22.04/24.04 smoke tests pass.
- [ ] PowerShell 5.1/7, bash, zsh, tmux and Neovim compatibility checks pass.

Unchecked items are release gates. In particular, Ubuntu GUI/font/Chinese IME checks must remain unchecked until performed on real Ubuntu 22.04 and 24.04 systems.

The automated 100,000-line case is intentionally ignored in ordinary debug test runs. Run it with `cargo test -p matcha-terminal --release retains_configured_history_after_one_hundred_thousand_lines -- --ignored` and record elapsed time plus an idle-CPU observation before checking the performance gate.

The Linux CI smoke test requires the `libxkbcommon-x11-0` runtime package in addition to the xkbcommon development headers. A missing custom terminal font must show a fallback notice and render with the bundled font rather than silently selecting a proportional system face.

The Windows release-mode local-shell echo benchmark samples 100 post-warmup commands and requires P95 at or below 50 ms. The 2026-09-01 local result was 2.10 ms; CI checks correctness and byte preservation but does not enforce this host-performance threshold.
