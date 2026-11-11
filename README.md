# fpsxterm-tui

A keyboard-first, mouse-friendly SSH, terminal, file management, and SFTP transfer application for Ubuntu terminals.

## Requirements

- Ubuntu 24.04 or newer on x86_64
- Stable Rust toolchain (`rustup`)
- OpenSSH client (`ssh`) and a working `~/.ssh/config` / SSH agent for remote access
- A terminal emulator with xterm-compatible mouse and alternate-screen support

Build and run from the repository:

```sh
cargo run
```

For a local release build, run `cargo build --release` and install the resulting `target/release/fpsxterm-tui` binary somewhere on your `PATH` (for example, `~/.local/bin`).

Run checks:

```sh
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```

## Current controls

- `1`–`5` (when app focus is active): open a workspace directly; `Shift+Tab` next, `Ctrl+Shift+Tab` previous, `Ctrl+Tab` toggles the two most recently used workspaces (`F8` fallback)
- `Alt+Left` / `Alt+Right`: previous / next workspace
- Hosts: `a` add, `e` edit, `i` import a supported OpenSSH entry, `Delete` remove, `Enter` connect, `r` reload
- Terminal: `n` start a local shell, `Alt+1`–`Alt+9` (or `Ctrl+1`–`Ctrl+9` in terminals that reserve Alt+number) select a session, `Ctrl+]` return focus to the app
- Files: `Tab` switch local/remote panels, arrows select, `Enter` preview/open, double-click preview/open, `Ctrl+K` or right-click open file actions, `Space` multi-select, `Backspace` parent, `Ctrl+L` go to path, `F2` rename, `Delete` remove, `F5` refresh, `F6` transfer, `F7` create folder
- Transfers shows start time in Vietnam time (UTC+7), duration, source/destination endpoints and absolute file path; select a row and press `c` to cancel
- History keeps the latest 1,000 SSH attempts in the XDG state directory. Select an entry and press `Enter` or double-click to reconnect; `Delete` removes it.
- `Ctrl+Q` quits

Files open in a read-only viewer by default; choose Edit from the file actions menu to open the configured external terminal editor (`nano` by default). The configuration file is stored in the XDG config directory under `fpsxterm/fpsxterm-tui`; the app-managed SSH entries live under `~/.ssh/fpsxterm-tui/hosts.conf` and are included from `~/.ssh/config`.

Remote terminal and SFTP operations use OpenSSH configuration, SSH agent authentication, and strict known-host verification. The application does not store SSH passwords.
