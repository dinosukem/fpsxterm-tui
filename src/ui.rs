use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState,
        Tabs,
    },
};

use crate::app::{App, FileInputOperation, FileModal, HostModal, Screen};

pub fn terminal_content_rect(area: Rect) -> Rect {
    if area.height < 10 || area.width < 3 {
        return Rect::default();
    }
    Rect::new(
        1,
        6,
        area.width.saturating_sub(2),
        area.height.saturating_sub(9),
    )
}

pub fn workspace_tab_at(column: u16) -> Option<Screen> {
    let mut x = 1u16;
    for screen in Screen::ALL {
        let width = screen.title().len() as u16 + 2;
        if column >= x && column < x + width {
            return Some(screen);
        }
        x = x.saturating_add(width + 1);
    }
    None
}

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < 50 || area.height < 13 {
        frame.render_widget(
            Paragraph::new(
            "fpsxterm-tui needs a terminal at least 50 columns by 13 rows. Press Ctrl+Q to quit.",
            ),
            area,
        );
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(3),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(" fpsxterm-tui ").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        chunks[0],
    );
    let titles = Screen::ALL.map(|screen| Line::from(Span::raw(format!(" {} ", screen.title()))));
    let selected = Screen::ALL
        .iter()
        .position(|screen| *screen == app.screen)
        .unwrap_or(0);
    frame.render_widget(
        Tabs::new(titles)
            .select(selected)
            .block(Block::default().borders(Borders::ALL).title(" Workspace "))
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        chunks[1],
    );
    render_screen(frame, app, chunks[2]);
    if let Some(modal) = &app.host_modal {
        render_host_modal(frame, modal, area);
    }
    if let Some(modal) = &app.file_modal {
        render_file_modal(frame, modal, area);
    }
    let footer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(chunks[3]);
    frame.render_widget(
        Paragraph::new(app.status.as_str()).style(Style::default().fg(Color::Gray)),
        footer[0],
    );
    let help = match app.screen {
        Screen::Hosts => "Enter connect · a add · i import · e edit · Delete remove · r refresh",
        Screen::Terminal => {
            "Ctrl+] app focus · n local shell · Alt+1..9 or Ctrl+1..9 sessions · x close"
        }
        Screen::Files => {
            "Tab panels · Enter view/open · Ctrl+K or right-click actions · F2 rename · Delete remove · F6 transfer · Ctrl+L path"
        }
        Screen::Transfers => "↑/↓ select · c cancel selected transfer",
        Screen::History => "↑/↓ select · Enter reconnect · Delete remove entry",
    };
    frame.render_widget(Paragraph::new(help), footer[1]);
    frame.render_widget(Paragraph::new("Ctrl+Tab recent · Shift+Tab next · Ctrl+Shift+Tab previous · Alt+←/→ previous/next · F8 recent · Ctrl+Q quit"), footer[2]);
}

fn render_file_modal(frame: &mut Frame<'_>, modal: &FileModal, area: Rect) {
    match modal {
        FileModal::Input {
            operation, value, ..
        } => {
            let (title, hint) = match operation {
                FileInputOperation::Rename(_) => (" Rename ", "Enter saves · Esc cancels"),
                FileInputOperation::CreateDirectory(_) => {
                    (" New Folder ", "Enter creates · Esc cancels")
                }
            };
            let rect = centered(64, 7, area);
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Paragraph::new(format!("{hint}\n\n{value}█"))
                    .block(Block::default().borders(Borders::ALL).title(title)),
                rect,
            );
        }
        FileModal::ConfirmDelete { entries, .. } => {
            let rect = centered(66, 8, area);
            frame.render_widget(Clear, rect);
            let names = entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            frame.render_widget(
                Paragraph::new(format!(
                    "Delete {} selected item(s)?\n{}\nThis action cannot be undone.\n\nEnter confirms · Esc cancels",
                    entries.len(), names
                ))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Confirm Delete "),
                ),
                rect,
            );
        }
        FileModal::ConfirmOverwrite { entry, .. } => {
            let rect = centered(66, 8, area);
            frame.render_widget(Clear, rect);
            let (rename, choice) = match modal {
                FileModal::ConfirmOverwrite { rename, choice, .. } => (rename, *choice),
                _ => unreachable!(),
            };
            let options = [
                format!("Rename copy: {rename}"),
                "Overwrite existing".into(),
                "Skip this file".into(),
                "Cancel transfer".into(),
            ];
            let text = format!(
                "'{}' already exists at destination.\n\n{}\n{}\n{}\n{}\n\n←/→ choose · Enter confirm · type to edit rename",
                entry.name,
                mark(&options[0], choice == 0),
                mark(&options[1], choice == 1),
                mark(&options[2], choice == 2),
                mark(&options[3], choice == 3)
            );
            frame.render_widget(
                Paragraph::new(text).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" File Conflict "),
                ),
                rect,
            );
        }
        FileModal::PathInput { value, .. } => {
            let rect = centered(72, 7, area);
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Paragraph::new(format!(
                    "Ctrl+L path · Enter opens · Esc cancels\n\n{value}█"
                ))
                .block(Block::default().borders(Borders::ALL).title(" Go to Path ")),
                rect,
            );
        }
        FileModal::Viewer {
            title,
            content,
            offset,
        } => {
            let rect = centered(
                area.width.saturating_sub(4),
                area.height.saturating_sub(4),
                area,
            );
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Paragraph::new(content.as_str())
                    .scroll((*offset as u16, 0))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" Viewer: {title} ")),
                    ),
                rect,
            );
        }
        FileModal::ContextMenu { choice, .. } => {
            let rect = centered(48, 12, area);
            frame.render_widget(Clear, rect);
            let items = [
                "View / open",
                "Edit with configured terminal editor",
                "Transfer",
                "Rename",
                "New folder",
                "Delete",
                "Refresh",
            ];
            let text = items
                .iter()
                .enumerate()
                .map(|(i, item)| mark(item, i == *choice))
                .collect::<Vec<_>>()
                .join("\n");
            frame.render_widget(
                Paragraph::new(text).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" File actions "),
                ),
                rect,
            );
        }
    }
}

fn render_screen(frame: &mut Frame<'_>, app: &App, area: Rect) {
    match app.screen {
        Screen::Hosts => {
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
                .split(area);
            let items: Vec<ListItem<'_>> = if app.hosts.is_empty() {
                vec![ListItem::new("No configured hosts. Press a to add one.")]
            } else {
                app.hosts
                    .iter()
                    .map(|host| {
                        ListItem::new(format!(
                            "{}  {}",
                            if host.source == crate::ssh::HostSource::Managed {
                                "◆"
                            } else {
                                "○"
                            },
                            host.alias
                        ))
                    })
                    .collect()
            };
            let mut state = ListState::default();
            if !app.hosts.is_empty() {
                state.select(Some(app.selected_host));
            }
            frame.render_stateful_widget(
                List::new(items)
                    .block(Block::default().borders(Borders::ALL).title(" SSH Hosts "))
                    .highlight_symbol("› ")
                    .highlight_style(Style::default().fg(Color::Yellow)),
                columns[0],
                &mut state,
            );
            let detail = app.hosts.get(app.selected_host).map(|host| {
                let endpoint = host.host_name.as_deref().unwrap_or("(from OpenSSH config)");
                let user = host.user.as_deref().unwrap_or("user from config");
                let port = host.port.map(|p| format!(":{p}")).unwrap_or_default();
                let source = if host.source == crate::ssh::HostSource::Managed { "Managed by fpsxterm-tui" } else { "Read-only OpenSSH host" };
                format!("Alias: {}\nEndpoint: {}@{}{}\nSource: {}\n\nEnter connect\na add  e edit\ni import\nDel delete", host.alias, user, endpoint, port, source)
            }).unwrap_or_else(|| "Select a host or add one.\n\nExternal hosts are shown read-only.".into());
            frame.render_widget(
                Paragraph::new(detail).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Host Details "),
                ),
                columns[1],
            );
        }
        Screen::Terminal => {
            let terminal_area = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(area);
            let sessions = app.terminals.session_titles();
            let tabs = Line::from(
                sessions
                    .iter()
                    .enumerate()
                    .flat_map(|(index, (id, title))| {
                        let active = Some(*id) == app.terminals.active_id();
                        let label = format!(" {}:{} ", index + 1, title);
                        let style = if active {
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Gray)
                        };
                        [Span::styled(label, style), Span::raw(" ")]
                    })
                    .collect::<Vec<_>>(),
            );
            frame.render_widget(
                if sessions.is_empty() {
                    Paragraph::new("No sessions — press n to start a local shell")
                } else {
                    Paragraph::new(tabs)
                },
                terminal_area[0],
            );
            if let Some(screen) = app.terminals.active_screen() {
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(app.terminals.active_title().unwrap_or(" Terminal "));
                frame.render_widget(
                    tui_term::widget::PseudoTerminal::new(screen).block(block),
                    terminal_area[1],
                );
            } else {
                frame.render_widget(
                    Paragraph::new("No active session. Press n to start a local shell.")
                        .block(Block::default().borders(Borders::ALL).title(" Terminal ")),
                    terminal_area[1],
                );
            }
        }
        Screen::Files => render_files(frame, app, area),
        Screen::Transfers => render_transfers(frame, app, area),
        Screen::History => render_history(frame, app, area),
    }
}

fn render_files(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    for (side, panel_area) in [
        (crate::app::FilePanel::Local, columns[0]),
        (crate::app::FilePanel::Remote, columns[1]),
    ] {
        let active = app.files.active == side;
        let (path, entries, selected, loading, title) = match side {
            crate::app::FilePanel::Local => (
                &app.files.local_path,
                &app.files.local_entries,
                app.files.selected_local,
                app.files.loading_local,
                " Local ",
            ),
            crate::app::FilePanel::Remote => (
                &app.files.remote_path,
                &app.files.remote_entries,
                app.files.selected_remote,
                app.files.loading_remote,
                " Remote ",
            ),
        };
        let title = match side {
            crate::app::FilePanel::Local => format!("{}{}", title, path.display()),
            crate::app::FilePanel::Remote => format!(
                "{}{}:{}",
                title,
                app.files.remote_host.as_deref().unwrap_or("select host"),
                path.display()
            ),
        };
        let mut items = Vec::with_capacity(entries.len() + 1);
        let multi_selected = match side {
            crate::app::FilePanel::Local => &app.files.selected_local_paths,
            crate::app::FilePanel::Remote => &app.files.selected_remote_paths,
        };
        if path.parent().is_some() {
            items.push(ListItem::new("📁 .."));
        }
        items.extend(entries.iter().map(|entry| {
            let (icon, size) = match entry.kind {
                crate::filesystem::EntryKind::Directory => ("📁", "<DIR>".to_owned()),
                crate::filesystem::EntryKind::Symlink => ("↪", "<LINK>".to_owned()),
                crate::filesystem::EntryKind::File => ("  ", format_size(entry.size.unwrap_or(0))),
                crate::filesystem::EntryKind::Other => ("?", "".to_owned()),
            };
            let marker = if multi_selected.contains(&entry.path) {
                "✓"
            } else {
                " "
            };
            ListItem::new(format!("{marker}{icon} {:<26} {:>8}", entry.name, size)).style(
                if multi_selected.contains(&entry.path) {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                },
            )
        }));
        if loading {
            items.insert(0, ListItem::new("Loading…"));
        }
        if items.is_empty() {
            items.push(ListItem::new(if side == crate::app::FilePanel::Remote {
                "No entries. Press r to refresh."
            } else {
                "Empty directory"
            }));
        }
        let selected = selected + usize::from(loading);
        let mut state = ListState::default();
        if !entries.is_empty() {
            state.select(Some(selected));
        }
        let border = if active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(border)
                        .title(title),
                )
                .highlight_symbol("› ")
                .highlight_style(Style::default().fg(Color::Yellow)),
            panel_area,
            &mut state,
        );
    }
}

fn render_transfers(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(65), Constraint::Min(3)])
        .split(area);
    let rows = app
        .transfers
        .iter()
        .map(|transfer| {
            let duration = if transfer.state == "Running" {
                transfer.elapsed.elapsed()
            } else {
                transfer.duration
            };
            let progress = transfer
                .total_bytes
                .map(|total| {
                    format!(
                        "{}%",
                        transfer
                            .bytes
                            .saturating_mul(100)
                            .checked_div(total.max(1))
                            .unwrap_or(0)
                            .min(100)
                    )
                })
                .unwrap_or_else(|| format!("{} B", transfer.bytes));
            Row::new(vec![
                Cell::from(format_vn(transfer.started_at)),
                Cell::from(format!("{}s", duration.as_secs())),
                Cell::from(transfer.source.clone()),
                Cell::from(transfer.destination.clone()),
                Cell::from(transfer.file_path.display().to_string()),
                Cell::from(format!("{} {progress}", transfer.state)),
            ])
        })
        .collect::<Vec<_>>();
    let header = Row::new([
        "Time (VN)",
        "Duration",
        "Source",
        "Destination",
        "File",
        "Status / progress",
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = TableState::default();
    if !app.transfers.is_empty() {
        state.select(Some(app.selected_transfer));
    }
    frame.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Length(20),
                Constraint::Length(10),
                Constraint::Percentage(22),
                Constraint::Percentage(22),
                Constraint::Percentage(27),
                Constraint::Percentage(15),
            ],
        )
        .header(header)
        .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::Yellow))
        .block(Block::default().borders(Borders::ALL).title(" Transfers ")),
        chunks[0],
        &mut state,
    );
    let details = app
        .transfers
        .get(app.selected_transfer)
        .map(|transfer| {
            format!(
                "File: {}\nSource: {}\nDestination: {}",
                transfer.file_path.display(),
                transfer.source,
                transfer.destination
            )
        })
        .unwrap_or_else(|| "No transfers yet. Start one with F6 in Files.".into());
    frame.render_widget(
        Paragraph::new(details).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Transfer details "),
        ),
        chunks[1],
    );
}

fn render_history(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let items = if app.history.is_empty() {
        vec![ListItem::new("No SSH connections yet.")]
    } else {
        app.history
            .iter()
            .map(|entry| {
                ListItem::new(format!(
                    "{}  {}  {}",
                    format_unix_vn(entry.at_unix),
                    entry.alias,
                    entry.status
                ))
            })
            .collect()
    };
    let mut state = ListState::default();
    if !app.history.is_empty() {
        state.select(Some(app.selected_history));
    }
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(area);
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" SSH History · newest first "),
            )
            .highlight_symbol("› ")
            .highlight_style(Style::default().fg(Color::Yellow)),
        columns[0],
        &mut state,
    );
    let details = app
        .history
        .get(app.selected_history)
        .map(|entry| {
            format!(
                "Host: {}\nTime: {}\nStatus: {}\n\nEnter reconnect\nDelete remove",
                entry.alias,
                format_unix_vn(entry.at_unix),
                entry.status
            )
        })
        .unwrap_or_else(|| {
            "History stores host alias and result only; credentials are not saved.".into()
        });
    frame.render_widget(
        Paragraph::new(details).block(Block::default().borders(Borders::ALL).title(" Details ")),
        columns[1],
    );
}

fn format_vn(time: std::time::SystemTime) -> String {
    let seconds = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    format_unix_vn(seconds as u64)
}

fn format_unix_vn(seconds: u64) -> String {
    let Ok(time) = time::OffsetDateTime::from_unix_timestamp(seconds as i64) else {
        return "-".into();
    };
    let local = time.to_offset(time::UtcOffset::from_hms(7, 0, 0).expect("UTC+7 is valid"));
    local
        .format(&time::macros::format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second]"
        ))
        .unwrap_or_else(|_| "-".into())
}

fn mark(text: &str, selected: bool) -> String {
    if selected {
        format!("› {text}")
    } else {
        format!("  {text}")
    }
}

fn format_size(size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = size as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn render_host_modal(frame: &mut Frame<'_>, modal: &HostModal, area: Rect) {
    match modal {
        HostModal::Form(form) => {
            let rect = centered(72, 13, area);
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .title(if form.old_alias.is_some() {
                        " Edit SSH Host "
                    } else {
                        " Add SSH Host "
                    }),
                rect,
            );
            let inner = rect.inner(Margin::new(1, 1));
            let fields = [
                ("Host alias", form.alias.as_str()),
                ("HostName", form.host_name.as_str()),
                ("User", form.user.as_str()),
                ("Port", form.port.as_str()),
                ("IdentityFile (optional)", form.identity_file.as_str()),
            ];
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(inner);
            frame.render_widget(
                Paragraph::new("Tab changes field · Enter advances/saves · Esc cancels"),
                rows[0],
            );
            for (index, ((label, value), row)) in fields.iter().zip(rows.iter().skip(1)).enumerate()
            {
                let style = if index == form.field {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                frame.render_widget(
                    Paragraph::new(format!("{label}: {value}")).style(style),
                    *row,
                );
            }
            frame.render_widget(Paragraph::new("Save: Enter on final field"), rows[6]);
        }
        HostModal::ConfirmDelete(alias) => {
            let rect = centered(58, 7, area);
            frame.render_widget(Clear, rect);
            frame.render_widget(Paragraph::new(format!("Delete managed SSH host '{alias}'?\nThe OpenSSH config and remote server are not deleted.\n\nEnter confirm · Esc cancel")).block(Block::default().borders(Borders::ALL).title(" Confirm Delete ")), rect);
        }
    }
}

fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
