use super::*;

pub(super) fn render(frame: &mut Frame<'_>, app: &App, area: Rect) {
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
    let detail = app
        .hosts
        .get(app.selected_host)
        .map(|host| {
            let endpoint = host.host_name.as_deref().unwrap_or("(from OpenSSH config)");
            let user = host.user.as_deref().unwrap_or("user from config");
            let port = host.port.map(|p| format!(":{p}")).unwrap_or_default();
            let source = if host.source == crate::ssh::HostSource::Managed {
                "Managed by fpsxterm-tui"
            } else {
                "Read-only OpenSSH host"
            };
            format!(
                "Alias: {}\nEndpoint: {}@{}{}\nSource: {}\n\nEnter connect\na add  e edit\ni import\nDel delete",
                host.alias, user, endpoint, port, source
            )
        })
        .unwrap_or_else(|| {
            "Select a host or add one.\n\nExternal hosts are shown read-only.".into()
        });
    frame.render_widget(
        Paragraph::new(detail).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Host Details "),
        ),
        columns[1],
    );
}
