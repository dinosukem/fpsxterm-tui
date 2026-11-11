use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;
use tokio::time::interval;

use fpsxterm_tui::{
    app::{App, Screen},
    config::ConfigStore,
    event::Action,
    ssh::HostStore,
    ui,
};

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        );
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("fpsxterm_tui=info".parse()?),
        )
        .with_writer(std::io::stderr)
        .init();

    let config_store = ConfigStore::discover()?;
    let config = config_store.load()?;
    let managed_dir = config_store.ssh_dir();
    let ssh_home = managed_dir
        .parent()
        .context("managed SSH directory has no parent")?
        .to_path_buf();
    let host_store = HostStore::new(&ssh_home, managed_dir);
    let hosts = host_store.list().unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read SSH hosts at startup");
        Vec::new()
    });
    enable_raw_mode().context("enabling terminal raw mode")?;
    let _guard = TerminalGuard;
    execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )
    .context("initializing terminal screen")?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let (runtime_tx, runtime_rx) = mpsc::channel(64);
    run(
        &mut terminal,
        App::new(
            config,
            runtime_tx,
            host_store,
            hosts,
            config_store.history_path(),
        ),
        runtime_rx,
    )
    .await
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    mut app: App,
    mut runtime_rx: mpsc::Receiver<fpsxterm_tui::terminal::RuntimeEvent>,
) -> Result<()> {
    let mut events = EventStream::new();
    let mut redraw = interval(Duration::from_millis(33));
    while !app.should_quit {
        tokio::select! {
            _ = redraw.tick() => {
                let size = terminal.size()?;
                let pty_area = ui::terminal_content_rect(ratatui::layout::Rect::new(0, 0, size.width, size.height));
                if app.screen == Screen::Terminal { app.resize_terminal(pty_area.height, pty_area.width); }
                for (id, status) in app.terminals.poll_exits() { app.record_terminal_exit(id, &status); }
                terminal.draw(|frame| { app.set_viewport(frame.area()); app.set_terminal_rect(ui::terminal_content_rect(frame.area())); ui::render(frame, &app) })?;
            }
            maybe_event = events.next() => match maybe_event {
                Some(Ok(event)) => handle_event(&mut app, event),
                Some(Err(error)) => return Err(error.into()),
                None => app.update(Action::Quit),
            },
            runtime = runtime_rx.recv() => if let Some(event) = runtime { app.process_runtime_event(event); },
        }
    }
    Ok(())
}

fn handle_event(app: &mut App, event: Event) {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if let Some(modal) = &app.file_modal {
                let is_delete = matches!(modal, fpsxterm_tui::app::FileModal::ConfirmDelete { .. });
                let is_overwrite =
                    matches!(modal, fpsxterm_tui::app::FileModal::ConfirmOverwrite { .. });
                let is_path = matches!(modal, fpsxterm_tui::app::FileModal::PathInput { .. });
                let is_viewer = matches!(modal, fpsxterm_tui::app::FileModal::Viewer { .. });
                let context_choice = match modal {
                    fpsxterm_tui::app::FileModal::ContextMenu { choice, .. } => Some(*choice),
                    _ => None,
                };
                let overwrite_choice = match modal {
                    fpsxterm_tui::app::FileModal::ConfirmOverwrite { choice, .. } => Some(*choice),
                    _ => None,
                };
                let is_confirm = is_delete || is_overwrite;
                match key.code {
                    KeyCode::Esc => app.update(Action::CancelFileDialog),
                    KeyCode::Up if is_viewer => app.update(Action::ScrollViewer(-1)),
                    KeyCode::Down if is_viewer => app.update(Action::ScrollViewer(1)),
                    KeyCode::PageUp if is_viewer => app.update(Action::ScrollViewer(-20)),
                    KeyCode::PageDown if is_viewer => app.update(Action::ScrollViewer(20)),
                    KeyCode::Up | KeyCode::Left if is_overwrite || context_choice.is_some() => {
                        app.update(Action::ChooseFileOption(-1))
                    }
                    KeyCode::Down | KeyCode::Right if is_overwrite || context_choice.is_some() => {
                        app.update(Action::ChooseFileOption(1))
                    }
                    KeyCode::Up if context_choice.is_some() => {
                        app.update(Action::ChooseFileOption(-1))
                    }
                    KeyCode::Down if context_choice.is_some() => {
                        app.update(Action::ChooseFileOption(1))
                    }
                    KeyCode::Enter if let Some(choice) = context_choice => {
                        app.update(Action::ContextAction(choice))
                    }
                    KeyCode::Backspace => app.update(Action::BackspaceFileInput),
                    KeyCode::Enter if is_delete => app.update(Action::ConfirmFileDelete),
                    KeyCode::Enter if is_overwrite => app.update(Action::ConfirmFileOverwrite),
                    KeyCode::Enter if is_path => app.update(Action::ApplyCurrentPath),
                    KeyCode::Enter => app.update(Action::SaveFileInput),
                    KeyCode::Char(ch)
                        if (!is_confirm || overwrite_choice == Some(0))
                            && !is_viewer
                            && context_choice.is_none()
                            && !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        app.update(Action::InsertFileChar(ch))
                    }
                    _ => {}
                }
                return;
            }
            if app.host_modal.is_some() {
                let confirm_delete = matches!(
                    app.host_modal,
                    Some(fpsxterm_tui::app::HostModal::ConfirmDelete(_))
                );
                let form_field = match &app.host_modal {
                    Some(fpsxterm_tui::app::HostModal::Form(form)) => Some(form.field),
                    _ => None,
                };
                match key.code {
                    KeyCode::Esc => app.update(Action::CancelDialog),
                    KeyCode::Tab => app.update(Action::NextDialogField(1)),
                    KeyCode::BackTab => app.update(Action::NextDialogField(-1)),
                    KeyCode::Backspace => app.update(Action::BackspaceDialog),
                    KeyCode::Enter if confirm_delete => app.update(Action::ConfirmDelete),
                    KeyCode::Enter if form_field.is_some_and(|field| field < 4) => {
                        app.update(Action::NextDialogField(1))
                    }
                    KeyCode::Enter => app.update(Action::SaveDialog),
                    KeyCode::Char(ch)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        app.update(Action::InsertDialogChar(ch))
                    }
                    _ => {}
                }
                return;
            }
            if key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.update(Action::RecentScreen);
                return;
            }
            if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.update(Action::Quit);
                return;
            }
            if key.code == KeyCode::BackTab && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.update(Action::PreviousScreen);
                return;
            }
            if key.code == KeyCode::BackTab
                || (key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT))
            {
                app.update(Action::NextScreen);
                return;
            }
            if key.code == KeyCode::F(8) {
                app.update(Action::RecentScreen);
                return;
            }
            if key.code == KeyCode::Left && key.modifiers.contains(KeyModifiers::ALT) {
                app.update(Action::PreviousScreen);
                return;
            }
            if key.code == KeyCode::Right && key.modifiers.contains(KeyModifiers::ALT) {
                app.update(Action::NextScreen);
                return;
            }
            if let KeyCode::Char(ch @ '1'..='9') = key.code
                && (key.modifiers.contains(KeyModifiers::ALT)
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && app.screen == Screen::Terminal))
                && app.screen == Screen::Terminal
            {
                app.select_terminal_index(usize::from(ch as u8 - b'1'));
                app.update(Action::Focus(
                    fpsxterm_tui::input::focus::FocusTarget::Terminal,
                ));
                return;
            }
            if key.code == KeyCode::Char(']') && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.update(Action::Focus(
                    fpsxterm_tui::input::focus::FocusTarget::HostList,
                ));
                return;
            }
            if app.focus == fpsxterm_tui::input::focus::FocusTarget::Terminal
                && app.terminals.active_id().is_some()
            {
                let input = key_to_bytes(key, app.terminals.active_application_cursor());
                if !input.is_empty() {
                    app.update(Action::SendTerminalInput(input));
                }
                return;
            }
            if app.screen == Screen::Files {
                if key.code == KeyCode::Char('k') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    app.update(Action::OpenFileContextMenu);
                    return;
                }
                if key.code == KeyCode::Char('l') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    app.update(Action::EditCurrentPath);
                    return;
                }
                match key.code {
                    KeyCode::Tab if key.modifiers.is_empty() => {
                        let side = if app.files.active == fpsxterm_tui::app::FilePanel::Local {
                            fpsxterm_tui::app::FilePanel::Remote
                        } else {
                            fpsxterm_tui::app::FilePanel::Local
                        };
                        app.update(Action::FocusFilePanel(side));
                    }
                    KeyCode::Enter => app.update(Action::OpenFileViewer),
                    KeyCode::Backspace => app.update(Action::ParentDirectory),
                    KeyCode::F(5) | KeyCode::Char('r') => app.update(Action::Refresh),
                    KeyCode::F(6) => app.update(Action::TransferSelected),
                    KeyCode::F(2) => app.update(Action::RenameSelected),
                    KeyCode::F(7) => app.update(Action::NewDirectory),
                    KeyCode::Char(' ') => app.update(Action::ToggleFileSelection),
                    KeyCode::Delete => app.update(Action::DeleteSelected),
                    KeyCode::Esc => app.update(Action::SelectScreen(Screen::Hosts)),
                    _ => {}
                }
                if (key.code == KeyCode::Tab && key.modifiers.is_empty())
                    || matches!(
                        key.code,
                        KeyCode::Enter
                            | KeyCode::Backspace
                            | KeyCode::F(5)
                            | KeyCode::F(6)
                            | KeyCode::F(2)
                            | KeyCode::F(7)
                            | KeyCode::Delete
                            | KeyCode::Char('r')
                            | KeyCode::Esc
                    )
                {
                    return;
                }
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                app.update(Action::Quit);
                return;
            }
            match key.code {
                KeyCode::Char('q') => app.update(Action::Quit),
                KeyCode::Tab => app.update(Action::NextScreen),
                KeyCode::BackTab => app.update(Action::PreviousScreen),
                KeyCode::Up => app.update(Action::MoveSelection(-1)),
                KeyCode::Down => app.update(Action::MoveSelection(1)),
                KeyCode::Enter if app.screen == Screen::Hosts => app.update(Action::ConnectHost),
                KeyCode::Enter if app.screen == Screen::History => {
                    app.update(Action::ConnectHistory)
                }
                KeyCode::Char('n') if app.screen == Screen::Terminal => {
                    app.update(Action::OpenLocalTerminal)
                }
                KeyCode::Char('c') if app.screen == Screen::Transfers => {
                    app.update(Action::CancelTransfer)
                }
                KeyCode::Char('x') if app.screen == Screen::Terminal => {
                    app.update(Action::CloseTerminal)
                }
                KeyCode::Delete if app.screen == Screen::History => {
                    app.update(Action::DeleteHistory)
                }
                KeyCode::Char('1' | '2' | '3' | '4' | '5') => {
                    let index = match key.code {
                        KeyCode::Char('1') => 0,
                        KeyCode::Char('2') => 1,
                        KeyCode::Char('3') => 2,
                        KeyCode::Char('4') => 3,
                        _ => 4,
                    };
                    app.update(Action::SelectScreen(Screen::ALL[index]));
                }
                KeyCode::Char('r') if app.screen != Screen::Hosts => app.update(Action::Refresh),
                KeyCode::Char('a') if app.screen == Screen::Hosts => app.update(Action::NewHost),
                KeyCode::Char('e') if app.screen == Screen::Hosts => app.update(Action::EditHost),
                KeyCode::Char('i') if app.screen == Screen::Hosts => app.update(Action::ImportHost),
                KeyCode::Delete | KeyCode::Backspace if app.screen == Screen::Hosts => {
                    app.update(Action::DeleteHost)
                }
                KeyCode::Char('r') if app.screen == Screen::Hosts => {
                    app.update(Action::ReloadHosts)
                }
                _ => {}
            }
        }
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) if (1..=3).contains(&mouse.row) => {
                if let Some(screen) = ui::workspace_tab_at(mouse.column) {
                    app.update(Action::SelectScreen(screen));
                }
            }
            MouseEventKind::Down(MouseButton::Left)
                if app.screen == Screen::Hosts
                    && mouse.row >= 5
                    && mouse.column < app.viewport.width * 58 / 100 =>
            {
                let visible = usize::from(app.viewport.height.saturating_sub(9)).max(1);
                let offset = app.selected_host.saturating_add(1).saturating_sub(visible);
                let index = usize::from(mouse.row.saturating_sub(5)).saturating_add(offset);
                if index < app.hosts.len() {
                    app.selected_host = index;
                }
            }
            MouseEventKind::Down(MouseButton::Left) if app.screen == Screen::Files => {
                select_file_at(app, mouse.column, mouse.row, false)
            }
            MouseEventKind::Down(MouseButton::Right) if app.screen == Screen::Files => {
                select_file_at(app, mouse.column, mouse.row, true)
            }
            MouseEventKind::Down(MouseButton::Left) if app.screen == Screen::History => {
                select_history_at(app, mouse.column, mouse.row)
            }
            MouseEventKind::Down(MouseButton::Left) if app.screen == Screen::Transfers => {
                select_transfer_at(app, mouse.column, mouse.row)
            }
            kind if app.screen == Screen::Terminal
                && app
                    .terminal_rect
                    .contains(ratatui::layout::Position::new(mouse.column, mouse.row)) =>
            {
                if app.terminals.active_id().is_some() {
                    let mode = app.terminals.active_mouse_mode();
                    if mode != vt100::MouseProtocolMode::None {
                        let x = mouse.column.saturating_sub(app.terminal_rect.x) + 1;
                        let y = mouse.row.saturating_sub(app.terminal_rect.y) + 1;
                        let bytes = encode_mouse(
                            kind,
                            mouse.modifiers,
                            x,
                            y,
                            mode,
                            app.terminals.active_mouse_encoding(),
                        );
                        if !bytes.is_empty() {
                            app.update(Action::SendTerminalInput(bytes));
                        }
                    }
                }
                app.update(Action::Focus(
                    fpsxterm_tui::input::focus::FocusTarget::Terminal,
                ));
            }
            MouseEventKind::ScrollUp if app.screen == Screen::Hosts => {
                app.update(Action::MoveSelection(-3))
            }
            MouseEventKind::ScrollDown if app.screen == Screen::Hosts => {
                app.update(Action::MoveSelection(3))
            }
            _ => {}
        },
        Event::Resize(_, _) => {}
        _ => {}
    }
}

fn select_file_at(app: &mut App, column: u16, row: u16, context_menu: bool) {
    if row < 5 || row >= app.viewport.height.saturating_sub(4) {
        return;
    }
    let side = if column < app.viewport.width / 2 {
        fpsxterm_tui::app::FilePanel::Local
    } else {
        fpsxterm_tui::app::FilePanel::Remote
    };
    app.update(Action::FocusFilePanel(side));
    let loading = match side {
        fpsxterm_tui::app::FilePanel::Local => app.files.loading_local,
        fpsxterm_tui::app::FilePanel::Remote => app.files.loading_remote,
    };
    let selected = app
        .files
        .selected(side)
        .saturating_add(usize::from(loading));
    let visible = usize::from(app.viewport.height.saturating_sub(9)).max(1);
    let offset = selected.saturating_add(1).saturating_sub(visible);
    let relative = usize::from(row - 5)
        .saturating_add(offset)
        .saturating_sub(usize::from(loading));
    let has_parent = match side {
        fpsxterm_tui::app::FilePanel::Local => app.files.local_path.parent().is_some(),
        fpsxterm_tui::app::FilePanel::Remote => app.files.remote_path.parent().is_some(),
    };
    let count = app.files.entries(side).len() + usize::from(has_parent);
    if relative >= count {
        return;
    }
    match side {
        fpsxterm_tui::app::FilePanel::Local => app.files.selected_local = relative,
        fpsxterm_tui::app::FilePanel::Remote => app.files.selected_remote = relative,
    }
    if context_menu {
        app.last_file_click = None;
        app.update(Action::OpenFileContextMenu);
        return;
    }
    let double_click = app
        .last_file_click
        .is_some_and(|(at, old_column, old_row)| {
            old_column == column && old_row == row && at.elapsed().as_millis() <= 450
        });
    app.last_file_click = Some((std::time::Instant::now(), column, row));
    if double_click {
        app.update(Action::OpenFileViewer);
    }
}

fn select_history_at(app: &mut App, column: u16, row: u16) {
    if column >= app.viewport.width * 68 / 100
        || row < 5
        || row >= app.viewport.height.saturating_sub(4)
    {
        return;
    }
    let visible = usize::from(app.viewport.height.saturating_sub(9)).max(1);
    let offset = app
        .selected_history
        .saturating_add(1)
        .saturating_sub(visible);
    let index = usize::from(row - 5).saturating_add(offset);
    if index >= app.history.len() {
        return;
    }
    let double_click = app
        .last_file_click
        .is_some_and(|(at, old_column, old_row)| {
            old_column == column && old_row == row && at.elapsed().as_millis() <= 450
        });
    app.selected_history = index;
    app.last_file_click = Some((std::time::Instant::now(), column, row));
    if double_click {
        app.update(Action::ConnectHistory);
    }
}

fn select_transfer_at(app: &mut App, _column: u16, row: u16) {
    let body_height = app.viewport.height.saturating_sub(7);
    let table_height = u32::from(body_height) * 65 / 100;
    let visible = (table_height as usize).saturating_sub(3).max(1);
    if row < 6 || u32::from(row) >= 4 + table_height.saturating_sub(1) {
        return;
    }
    let offset = app
        .selected_transfer
        .saturating_add(1)
        .saturating_sub(visible);
    let index = usize::from(row - 6).saturating_add(offset);
    if index < app.transfers.len() {
        app.selected_transfer = index;
    }
}

fn key_to_bytes(key: crossterm::event::KeyEvent, application_cursor: bool) -> Vec<u8> {
    use KeyCode::*;
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let Char(c @ 'a'..='z') = key.code {
            return vec![c as u8 - b'a' + 1];
        }
        if let Char(c @ 'A'..='Z') = key.code {
            return vec![c as u8 - b'A' + 1];
        }
        return match key.code {
            Char(' ') => vec![0],
            Char('[') => vec![0x1b],
            Char('\\') => vec![0x1c],
            Char(']') => vec![0x1d],
            _ => vec![],
        };
    }
    let mut bytes = match key.code {
        Char(c) => {
            let mut buf = [0; 4];
            c.encode_utf8(&mut buf).as_bytes().to_vec()
        }
        Enter => vec![b'\r'],
        Tab => vec![b'\t'],
        Backspace => vec![0x7f],
        Esc => vec![0x1b],
        Up => {
            if application_cursor {
                b"\x1bOA".to_vec()
            } else {
                b"\x1b[A".to_vec()
            }
        }
        Down => {
            if application_cursor {
                b"\x1bOB".to_vec()
            } else {
                b"\x1b[B".to_vec()
            }
        }
        Right => {
            if application_cursor {
                b"\x1bOC".to_vec()
            } else {
                b"\x1b[C".to_vec()
            }
        }
        Left => {
            if application_cursor {
                b"\x1bOD".to_vec()
            } else {
                b"\x1b[D".to_vec()
            }
        }
        Home => b"\x1b[H".to_vec(),
        End => b"\x1b[F".to_vec(),
        PageUp => b"\x1b[5~".to_vec(),
        PageDown => b"\x1b[6~".to_vec(),
        Delete => b"\x1b[3~".to_vec(),
        Insert => b"\x1b[2~".to_vec(),
        F(n @ 1..=4) => format!("\x1b[{}~", 10 + n).into_bytes(),
        F(5) => b"\x1b[15~".to_vec(),
        F(6) => b"\x1b[17~".to_vec(),
        F(7) => b"\x1b[18~".to_vec(),
        F(8) => b"\x1b[19~".to_vec(),
        F(9) => b"\x1b[20~".to_vec(),
        F(10) => b"\x1b[21~".to_vec(),
        F(11) => b"\x1b[23~".to_vec(),
        F(12) => b"\x1b[24~".to_vec(),
        _ => vec![],
    };
    if key.modifiers.contains(KeyModifiers::ALT) && !bytes.is_empty() {
        bytes.insert(0, 0x1b);
    }
    bytes
}

fn encode_mouse(
    kind: MouseEventKind,
    modifiers: KeyModifiers,
    x: u16,
    y: u16,
    mode: vt100::MouseProtocolMode,
    encoding: vt100::MouseProtocolEncoding,
) -> Vec<u8> {
    use MouseEventKind as M;
    let (mut code, release) = match kind {
        M::Down(button) => (mouse_button(button), false),
        M::Up(button) if mode != vt100::MouseProtocolMode::Press => (mouse_button(button), true),
        M::Drag(button)
            if matches!(
                mode,
                vt100::MouseProtocolMode::ButtonMotion | vt100::MouseProtocolMode::AnyMotion
            ) =>
        {
            (mouse_button(button) + 32, false)
        }
        M::Moved if mode == vt100::MouseProtocolMode::AnyMotion => (35, false),
        M::ScrollUp => (64, false),
        M::ScrollDown => (65, false),
        M::ScrollLeft => (66, false),
        M::ScrollRight => (67, false),
        _ => return Vec::new(),
    };
    if modifiers.contains(KeyModifiers::SHIFT) {
        code += 4;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        code += 8;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        code += 16;
    }
    match encoding {
        vt100::MouseProtocolEncoding::Sgr => {
            format!("\x1b[<{code};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes()
        }
        vt100::MouseProtocolEncoding::Default | vt100::MouseProtocolEncoding::Utf8 => {
            if x > 223 || y > 223 || code > 223 {
                return Vec::new();
            }
            let code = if release { 3 + (code & !32) } else { code };
            vec![
                0x1b,
                b'[',
                b'M',
                32 + code as u8,
                32 + x as u8,
                32 + y as u8,
            ]
        }
    }
}

fn mouse_button(button: MouseButton) -> u16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}
