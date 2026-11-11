use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs},
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

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < 50 || area.height < 12 {
        frame.render_widget(
            Paragraph::new(
                "fpsxterm-tui needs a terminal at least 50 columns by 12 rows. Press q to quit.",
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
            Constraint::Min(5),
            Constraint::Length(2),
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
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(chunks[3]);
    frame.render_widget(
        Paragraph::new(app.status.as_str()).style(Style::default().fg(Color::Gray)),
        footer[0],
    );
    let help = match app.screen {
        Screen::Hosts => {
            "1-4 screens · ↑/↓ select · Enter connect · a add · i import · e edit · Del delete · q quit"
        }
        Screen::Terminal => {
            "Ctrl+] app focus · n local shell · Alt+1..9 sessions · x close · q quit"
        }
        Screen::Files => {
            "Tab panels · Space select · Enter open · F2 rename · Del delete · F5 refresh · F6 transfer · F7 mkdir · Ctrl+L path"
        }
        Screen::Transfers => "c cancel latest transfer · 1-4 screens · q quit",
    };
    frame.render_widget(Paragraph::new(help), footer[1]);
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
            frame.render_widget(
                Paragraph::new(format!("'{}' already exists at the destination.\nReplace it?\n\nEnter confirms · Esc cancels", entry.name))
                    .block(Block::default().borders(Borders::ALL).title(" Confirm Overwrite ")),
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
            let tabs = app
                .terminals
                .session_titles()
                .into_iter()
                .map(|(id, title)| {
                    if Some(id) == app.terminals.active_id() {
                        format!(" [{}:{}] ", id, title)
                    } else {
                        format!(" {}:{} ", id, title)
                    }
                })
                .collect::<String>();
            frame.render_widget(
                Paragraph::new(if tabs.is_empty() {
                    "No sessions — press n to start a local shell".to_owned()
                } else {
                    tabs
                })
                .style(Style::default().fg(Color::Cyan)),
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
        Screen::Transfers => {
            let items = if app.transfers.is_empty() {
                vec![ListItem::new(
                    "Transfer queue is empty. Press F6 in Files to transfer a file or folder.",
                )]
            } else {
                app.transfers
                    .iter()
                    .map(|transfer| {
                        let progress = transfer
                            .total_bytes
                            .map(|total| {
                                let percent = transfer
                                    .bytes
                                    .saturating_mul(100)
                                    .checked_div(total)
                                    .unwrap_or(100)
                                    .min(100);
                                format!("{}% ({} / {} bytes)", percent, transfer.bytes, total)
                            })
                            .unwrap_or_else(|| format!("{} bytes", transfer.bytes));
                        ListItem::new(format!(
                            "{}  [{}]  {progress}",
                            transfer.label, transfer.state
                        ))
                    })
                    .collect()
            };
            frame.render_widget(
                List::new(items).block(Block::default().borders(Borders::ALL).title(" Transfers ")),
                area,
            );
        }
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
            let rect = centered(64, 15, area);
            frame.render_widget(Clear, rect);
            let fields = [
                ("Host alias", form.alias.as_str()),
                ("HostName", form.host_name.as_str()),
                ("User", form.user.as_str()),
                ("Port", form.port.as_str()),
                ("IdentityFile (optional)", form.identity_file.as_str()),
            ];
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(rect);
            let title = if form.old_alias.is_some() {
                " Edit SSH Host "
            } else {
                " Add SSH Host "
            };
            frame.render_widget(
                Paragraph::new(title).style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                rows[0],
            );
            frame.render_widget(
                Paragraph::new("Tab changes field · Enter advances/saves · Esc cancels"),
                rows[1],
            );
            for (index, ((label, value), row)) in fields.iter().zip(rows.iter().skip(2)).enumerate()
            {
                let style = if index == form.field {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                frame.render_widget(
                    Paragraph::new(format!("{label}: {value}"))
                        .style(style)
                        .block(Block::default().borders(Borders::ALL)),
                    *row,
                );
            }
            frame.render_widget(Paragraph::new("Save: Enter on final field"), rows[7]);
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
