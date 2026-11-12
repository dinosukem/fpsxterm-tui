use super::*;

pub(super) fn render(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    let tree_rows = app.host_tree_rows();
    let items: Vec<ListItem<'_>> = if tree_rows.is_empty() {
        vec![ListItem::new("No configured hosts. Press A to add one.")]
    } else {
        tree_rows
            .iter()
            .map(|row| {
                let host = &app.hosts[row.host_index];
                let icon = match host.source {
                    crate::ssh::HostSource::Managed => "◆",
                    crate::ssh::HostSource::OpenSsh => "○",
                    crate::ssh::HostSource::Docker => "🐳",
                };
                let branch = if row.has_children {
                    if row.expanded { "▾" } else { "▸" }
                } else {
                    " "
                };
                let item = ListItem::new(format!(
                    "{}{} {} {}",
                    "  ".repeat(row.depth),
                    branch,
                    icon,
                    host.alias
                ));
                if app.hovered_host == Some(row.host_index) && row.host_index != app.selected_host {
                    item.style(Style::default().bg(Color::Gray).fg(Color::Black))
                } else {
                    item
                }
            })
            .collect()
    };
    let selected_position = tree_rows
        .iter()
        .position(|row| row.host_index == app.selected_host);
    let mut state = ListState::default();
    state.select(selected_position);
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" SSH Hosts "))
            .highlight_symbol("› ")
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default()),
        columns[0],
        &mut state,
    );

    let detail: Vec<Line<'static>> = app
        .hosts
        .get(app.selected_host)
        .map(|host| {
            let mut lines = Vec::new();
            if let Some(docker) = &host.docker {
                lines.push(Line::from(format!(
                    "Docker container: {}",
                    docker.container
                )));
                lines.push(Line::from(format!("Remote SSH host: {}", docker.ssh_host)));
                lines.push(Line::from(format!("Shell: {:?}", docker.shell)));
                if !app.hosts.iter().any(|candidate| {
                    candidate.source != crate::ssh::HostSource::Docker
                        && candidate.alias == docker.ssh_host
                }) {
                    lines.push(Line::styled(
                        "Parent SSH host is missing from configuration",
                        Style::default().fg(Color::Red),
                    ));
                }
            } else {
                let endpoint = host.host_name.as_deref().unwrap_or("(from OpenSSH config)");
                let user = host.user.as_deref().unwrap_or("user from config");
                let port = host.port.map(|p| format!(":{p}")).unwrap_or_default();
                let source = if host.source == crate::ssh::HostSource::Managed {
                    "Managed by fpsxterm-tui"
                } else {
                    "Read-only OpenSSH host"
                };
                lines.push(Line::from(format!("Alias: {}", host.alias)));
                lines.push(Line::from(format!(
                    "Endpoint: {}@{}{}",
                    user, endpoint, port
                )));
                lines.push(Line::from(format!("Source: {source}")));
            }
            let metadata = app.host_metadata(host);
            lines.push(Line::from(format!(
                "Created: {}",
                metadata
                    .and_then(|value| value.created_at)
                    .map(super::format_unix_vn)
                    .unwrap_or_else(|| "Không rõ".into())
            )));
            lines.push(Line::from(format!(
                "Updated: {}",
                metadata
                    .and_then(|value| value.updated_at)
                    .map(super::format_unix_vn)
                    .unwrap_or_else(|| "Không rõ".into())
            )));
            lines.push(Line::from(""));

            if let Some(feedback) = &app.host_feedback
                && (feedback
                    .alias
                    .as_deref()
                    .is_none_or(|alias| alias == host.alias))
            {
                let color = match feedback.kind {
                    crate::app::HostFeedbackKind::Running => Color::Yellow,
                    crate::app::HostFeedbackKind::Success => Color::Green,
                    crate::app::HostFeedbackKind::Warning => Color::LightYellow,
                    crate::app::HostFeedbackKind::Error => Color::Red,
                };
                lines.push(Line::styled(
                    feedback.message.clone(),
                    Style::default().fg(color),
                ));
            } else if app.testing_host.as_deref() == Some(host.alias.as_str()) {
                lines.push(Line::styled(
                    "Test connection: running…",
                    Style::default().fg(Color::Yellow),
                ));
            } else if let Some((alias, result)) = &app.connection_test_result
                && alias == &host.alias
            {
                let (message, color) = match result {
                    Ok(details) => {
                        (
                            format!("Test connection: succeeded — {details}!"),
                            Color::Green,
                        )
                    }
                    Err(error) => (format!("Test connection: failed — {error}"), Color::Red),
                };
                lines.push(Line::styled(message, Style::default().fg(color)));
            }
            lines.push(Line::from(""));

            if host.source == crate::ssh::HostSource::Docker {
                lines.push(super::shortcut_line(&[
                    ("Enter", "connect"),
                    ("T", "test connection"),
                ]));
                lines.push(super::shortcut_line(&[("E", "edit"), ("Del", "remove")]));
            } else if host.source == crate::ssh::HostSource::OpenSsh {
                lines.push(super::shortcut_line(&[
                    ("Enter", "connect"),
                    ("T", "test connection"),
                ]));
                lines.push(super::shortcut_line(&[
                    ("I", "import to edit"),
                    ("R", "refresh"),
                ]));
            } else {
                lines.push(super::shortcut_line(&[
                    ("Enter", "connect"),
                    ("T", "test connection"),
                ]));
                lines.push(super::shortcut_line(&[
                    ("A", "add"),
                    ("E", "edit"),
                    ("I", "import"),
                ]));
                lines.push(super::shortcut_line(&[
                    ("D", "add Docker"),
                    ("L", "list containers"),
                    ("Del", "delete"),
                ]));
            }
            lines
        })
        .unwrap_or_else(|| {
            vec![
                Line::from("Select a host or add one."),
                Line::from(""),
                Line::from("External hosts are shown read-only."),
            ]
        });
    frame.render_widget(
        Paragraph::new(detail)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Host Details "),
            ),
        columns[1],
    );
}
