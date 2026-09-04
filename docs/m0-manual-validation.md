# Matcha M0 manual validation

Use a release build for every performance or compatibility result. Record the OS version, shell/tool version, display scale, locale, and whether the result is native or virtualized.

## Automated release stress case

```text
cargo test -p matcha-terminal --release retains_configured_history_after_one_hundred_thousand_lines -- --ignored
```

The test feeds 100,000 lines into a 120×40 terminal, verifies that exactly 50,000 history rows remain, and verifies that subsequent input is still processed. It does not measure the GUI frame rate or idle CPU.

## Windows 10 and 11

1. Start each discovered profile: PowerShell 7, Windows PowerShell 5.1, and cmd.
2. For each shell, enter Unicode text, resize repeatedly, run a command that exits with a non-zero code, and use Matcha's restart action. Confirm that prior output and the restart separator remain.
3. Run a 100,000-line command and confirm the window accepts keyboard input throughout.
4. Exercise `Ctrl+Shift+C/V`, multi-line paste confirmation, search navigation, font zoom, mouse selection, and selection-with-Shift while an application has mouse tracking enabled.
5. Verify Chinese IME preedit is drawn at the cursor and the committed text is sent exactly once.
6. Leave an unchanged terminal visible for two minutes and measure Matcha's CPU use. Record the observed range; do not mark idle CPU passed from visual inspection alone.
7. Compare the default 14 px / 1.15 line-height rendering beside Windows Terminal. Check ASCII tables, PowerShell prompts, box drawing, Chinese, emoji, combining accents, bold/italic text, and 125%/150% display scaling. Repeat after changing line height to 1.0 and 1.5; cursor, selection, search highlights, mouse hit testing, and PTY dimensions must remain aligned.
8. Use a prompt with adjacent colored background cells. At 100%, 125%, and 150% scaling, verify there are no dark seams between cells or inside single-line, multi-line, and reversed selections. Type commands containing repeated spaces quickly and confirm every space reaches the shell in order.
9. Use the repository's Starship prompt and compare ``, ``, `󰍲`, ``, ``, and `` beside Windows Terminal. Confirm every symbol maps correctly and does not shift following text at 100%, 125%, and 150% scaling.
10. Trigger a PSReadLine history prediction. Confirm the ghost completion is visibly dimmer than typed input, then accept it and confirm the resulting command uses normal foreground intensity.
11. Open settings from the gear and `Ctrl+,` at 640×480, the default size, and maximized. Confirm it fills the client area, scrolls, receives text input without leaking keys to the shell, closes with `Esc`, `Ctrl+,`, and the close button, and immediately restores terminal input focus.

## Ubuntu 22.04 and 24.04

1. Repeat the common terminal, resize, history, search, zoom, clipboard, multi-line paste, and IME checks under both X11 and Wayland when available.
2. Test the configured login shell plus installed bash and zsh profiles.
3. In tmux and Neovim, verify alternate-screen entry/exit, application cursor keys, wheel/mouse reporting, and Shift-forced local selection/scrolling.
4. Confirm JetBrains Mono NL and Symbols Nerd Font Mono load from the application assets and CJK glyphs use a system fallback without shifting cell boundaries.
5. Run the window smoke test under the CI virtual display, then repeat GUI, font, and Chinese IME validation on real machines. Virtual-display success is not a substitute for the real-machine items.

Only update [m0-validation.md](m0-validation.md) with measured results and the environment in which they were obtained.
