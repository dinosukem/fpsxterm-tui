use super::*;

pub(super) fn render_files(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    for (side, panel_area) in [
        (crate::app::FilePanel::Local, columns[0]),
        (crate::app::FilePanel::Remote, columns[1]),
    ] {
        let active = app.files.active == side;
        let has_terminal_session = app.terminals.active_id().is_some();
        let (path, entries, selected, loading, title) = match side {
            crate::app::FilePanel::Local => {
                (
                    &app.files.local_path,
                    &app.files.local_entries,
                    app.files.selected_local,
                    app.files.loading_local,
                    " Local ",
                )
            }
            crate::app::FilePanel::Remote => {
                (
                    &app.files.remote_path,
                    &app.files.remote_entries,
                    app.files.selected_remote,
                    app.files.loading_remote,
                    " Remote ",
                )
            }
        };
        let active_remote = app.terminals.active_remote();
        let title = match side {
            crate::app::FilePanel::Local => format!("{}{}", title, path.display()),
            crate::app::FilePanel::Remote => {
                let target = active_remote
                    .as_ref()
                    .map(|remote| remote.ssh_host.as_str())
                    .unwrap_or("no SSH session");
                format!("{title}{target}:{}", path.display())
            }
        };
        let mut items = Vec::with_capacity(entries.len() + 1);
        let multi_selected = match side {
            crate::app::FilePanel::Local => &app.files.selected_local_paths,
            crate::app::FilePanel::Remote => &app.files.selected_remote_paths,
        };
        if has_terminal_session && path.parent().is_some() {
            items.push(ListItem::new("📁 .."));
        }
        if has_terminal_session {
            items.extend(entries.iter().map(|entry| {
                let (icon, size) = match entry.kind {
                    crate::filesystem::EntryKind::Directory => ("📁", "<DIR>".to_owned()),
                    crate::filesystem::EntryKind::Symlink => ("↪", "<LINK>".to_owned()),
                    crate::filesystem::EntryKind::File => {
                        ("  ", format_size(entry.size.unwrap_or(0)))
                    }
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
        }
        if loading {
            items.insert(0, ListItem::new("Loading…"));
        }
        if items.is_empty() {
            let message = if app.terminals.active_id().is_none() {
                "Open or connect a terminal before browsing files"
            } else if side == crate::app::FilePanel::Remote
                && active_remote
                    .as_ref()
                    .is_some_and(|remote| remote.container.is_some())
            {
                "No entries. Check status for errors or press r to retry."
            } else if side == crate::app::FilePanel::Remote && active_remote.is_none() {
                "Remote Files require an SSH terminal session"
            } else if side == crate::app::FilePanel::Remote {
                "No entries. Press r to refresh."
            } else {
                "Empty directory"
            };
            items.push(ListItem::new(message));
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

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            app::App,
            config::{
                AppConfig,
                ConfigStore,
            },
            ssh::HostStore,
            terminal::RuntimeEvent,
        },
        ratatui::{
            Terminal,
            backend::TestBackend,
        },
        tokio::sync::mpsc,
    };

    #[test]
    fn files_without_a_terminal_session_do_not_show_local_entries() {
        let (tx, _rx) = mpsc::channel::<RuntimeEvent>(4);
        let dir = tempfile::tempdir().unwrap();
        let ssh_home = dir.path().join(".ssh");
        let mut app = App::new(
            AppConfig::default(),
            tx,
            HostStore::new(&ssh_home, ssh_home.join("fpsxterm-tui")),
            Vec::new(),
            dir.path().join("history.toml"),
            ConfigStore::from_paths(
                dir.path().join("config.toml"),
                ssh_home.join("fpsxterm-tui"),
                dir.path().join("state"),
            ),
        );
        app.files.local_entries.push(crate::filesystem::FileEntry {
            name: "local-only.txt".into(),
            path: app.files.local_path.join("local-only.txt"),
            kind: crate::filesystem::EntryKind::File,
            size: Some(12),
            modified: None,
        });
        let backend = TestBackend::new(160, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_files(frame, &app, frame.area()))
            .unwrap();

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!rendered.contains(".."));
        assert!(!rendered.contains("local-only.txt"));
        assert!(rendered.contains("Open or connect a terminal before browsing files"));
    }
}
