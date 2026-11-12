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

- `Alt+H` Hosts, `Alt+Shift+H` History, `Alt+T` Terminal, `Alt+Shift+T` Transfers, `Alt+F` Files, `Alt+C` Configuration; `Alt+Left` / `Alt+Right` move between adjacent workspaces; `F8` returns to the recent workspace
- Hosts: `a` add, `e` edit, `i` import a supported OpenSSH entry, `t` test connection, `d` add a Docker container host, `l` list running containers on the selected SSH host, `Delete` remove, `Enter` connect, `r` reload
- Hosts: Docker hosts appear beneath their SSH parent and can be collapsed or expanded with `Left`/`Right` or the mouse disclosure marker. Editing a managed SSH parent updates its Docker children; deleting it asks for confirmation and removes those app configurations too, without deleting remote containers or interrupting existing sessions. Created/updated times use Vietnam time; older entries without metadata show `Không rõ`.
- Hosts, Files, and Configuration support mouse-wheel navigation one row/field at a time. Files scrolls the pane under the pointer; Configuration scrolling changes selection only and does not edit values.
- Terminal: `n` start a local shell, `Alt+1`–`Alt+9` (or `Ctrl+1`–`Ctrl+9` in terminals that reserve Alt+number) select a session, `Ctrl+T` test the selected host, `Ctrl+]` return focus to the app
- Files: requires an active terminal session; SSH sessions show local and remote panes, while local sessions show local files and Docker sessions show the container's remote files. The pane paths are kept per terminal session. `Tab` switches file panes, arrows select, `Enter`/double-click opens the viewer, `Ctrl+Enter` opens the configured editor, `Ctrl+K` or right-click opens file actions, `Space` multi-select, `Backspace` parent, `Ctrl+L` go to path, `F2` rename, `Delete` remove, `F5` refresh, `F6` transfer, `F7` create folder
- Hosts: `Ctrl+T` tests the current add/edit form but is optional; hosts can be saved without a successful test. Docker container file access uses container utilities already installed; if required tools are missing, Files reports package-manager suggestions and never installs them automatically.
- Configuration: edit UI, terminal, Files, and transfer settings; `Space` toggles boolean values, `Enter` edits text/numbers, `Ctrl+A` clears the selected text/number field, and `Ctrl+S` validates, saves atomically, and reloads the settings. Theme currently supports `dark` only. Existing terminal sessions and active transfers keep running; updated terminal settings apply to new sessions.
- Transfers shows start time in Vietnam time (UTC+7), duration, source/destination endpoints and absolute file path; select a row and press `c` to cancel
- History keeps the latest 1,000 SSH attempts in the XDG state directory. Select an entry and press `Enter` or double-click to reconnect; `Delete` removes it.
- `Ctrl+Q` quits

Docker container hosts are saved in `config.toml` and appear as `container@ssh-host`. For example:

```toml
[[docker_hosts]]
ssh_host = "production"
container = "web"
shell = "auto" # auto, bash, or sh
```

They use the SSH user's Docker permissions and support terminal/history, remote file browsing and file transfers through `docker exec` over SSH. The default shell tries Bash and falls back to `sh`. The Hosts details panel shows operation and connection-test results, plus Vietnam-time creation/update timestamps for managed hosts.

In Files, `Enter` and double-click open the read-only viewer, while `Ctrl+Enter` opens the configured terminal editor (`nano` by default), including for Docker container files. `Ctrl+Shift+Tab` moves to the previous workspace, including while the terminal has focus. The configuration file is stored in the XDG config directory under `fpsxterm/fpsxterm-tui`; the app-managed SSH entries live under `~/.ssh/fpsxterm-tui/hosts.conf` and are included from `~/.ssh/config`.

Remote terminal and SFTP operations use OpenSSH configuration, SSH agent authentication, and strict known-host verification. The application does not store SSH passwords.
