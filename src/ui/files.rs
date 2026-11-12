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
