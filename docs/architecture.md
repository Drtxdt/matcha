# Matcha architecture

## M0 boundaries

Matcha starts with a single-session local terminal rather than a feature-complete remote workstation.

- `matcha-core` owns product-domain types and does not depend on UI or terminal crates.
- `matcha-config` owns versioned TOML configuration, recovery and platform shell discovery.
- `matcha-terminal` hides `alacritty_terminal` behind Matcha-owned frames, actions and input types.
- `matcha-pty` owns local process lifecycle and isolates blocking PTY reads/writes on dedicated threads.
- `matcha-ui` is the only crate allowed to expose Floem APIs.
- `matcha-app` wires logging and launches the native desktop process.

Future SSH channels will implement the same byte transport contract as the local PTY. UI code consumes terminal frames and actions and must never own blocking process or network work.

## Threading invariant

Floem stays on the platform UI thread. PTY reads and writes use dedicated workers. Session events cross a bounded channel; adjacent output notifications may be coalesced before the newest frame is extracted. The renderer receives an owned frame and does not hold the terminal lock while painting.

## Security defaults

Multiline paste requires confirmation and is encoded using bracketed paste when requested by the terminal. OSC 52 reads are denied. OSC 52 writes require a visible per-request decision unless the user explicitly trusts the current local profile. Selection does not copy unless the user enables that policy.
