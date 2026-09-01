# Matcha

Matcha is a native Rust terminal and future robotics development workstation.

The repository is currently in **M0 local-terminal validation**. It contains a single-session local terminal with a Floem workbench, an Alacritty-compatible terminal state machine, platform shell discovery, persistent settings, bilingual UI foundations, and guarded clipboard operations. SSH, SFTP, ROS, tabs, splits, completion, and plugins are intentionally outside M0.

## Development

```powershell
cargo run -p matcha-app
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

## M0 shortcuts

- `Ctrl+C`: send interrupt; `Ctrl+Shift+C/V`: copy/paste.
- `Ctrl+F`: search; `Enter`/`Shift+Enter`: next/previous result; `Esc`: close.
- `Ctrl+,`: settings; `Ctrl++/-/0`: terminal font size.
- Drag/double-click/triple-click selects characters/words/lines. Hold `Shift` to force local selection or scrolling while an application owns the mouse.

The default configuration is stored in the platform-standard Matcha configuration directory. Invalid TOML is backed up before defaults are restored.
The bundled `JetBrains Mono NL` font is the terminal default. Font size and compact line height can be adjusted independently; unavailable custom font names visibly fall back to the bundled font.

Supported desktop targets for the first release are Windows 10/11 and Ubuntu 22.04/24.04.

## License

Apache-2.0.
