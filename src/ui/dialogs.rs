use super::*;

pub(super) fn render_file_modal(frame: &mut Frame<'_>, modal: &FileModal, area: Rect) {
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
                Paragraph::new(vec![
                    Line::from(format!("{value}█")),
                    Line::from(""),
                    super::shortcut_line(&[
                        ("Tab", "complete folder"),
                        ("Enter", "open"),
                        ("Esc", "cancel"),
                    ]),
                ])
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
        FileModal::ContextMenu { choice, .. } => context_menu::render(frame, *choice, area),
        FileModal::CloseSessionTransfers { choice, .. } => {
            let rect = centered(72, 8, area);
            frame.render_widget(Clear, rect);
            let first = if *choice == 0 {
                "> Cancel transfers and close session"
            } else {
                "  Cancel transfers and close session"
            };
            let second = if *choice == 1 {
                "> Keep session open"
            } else {
                "  Keep session open"
            };
            frame.render_widget(
                Paragraph::new(format!("This terminal has active file transfers.\n\n{first}\n{second}\n\n↑/↓ choose · Enter confirm · Esc go back"))
                    .block(Block::default().borders(Borders::ALL).title(" Active Transfers ")),
                rect,
            );
        }
    }
}
pub(super) fn render_host_modal(
    frame: &mut Frame<'_>,
    modal: &HostModal,
    feedback: Option<&HostFeedback>,
    area: Rect,
) {
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
                Paragraph::new(super::shortcut_line(&[
                    ("Tab", "field"),
                    ("Ctrl+T", "test"),
                    ("Enter", "advance/save"),
                    ("Esc", "cancel"),
                ])),
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
            let fallback = if form.testing {
                "Testing connection…"
            } else if form.tested_revision == Some(form.revision) {
                "Connection succeeded! Test is optional"
            } else {
                "Connection test is optional"
            };
            let (message, color) = feedback
                .filter(|feedback| {
                    feedback
                        .alias
                        .as_deref()
                        .is_none_or(|alias| alias == form.alias)
                })
                .map(|feedback| {
                    let color = match feedback.kind {
                        HostFeedbackKind::Running => Color::Yellow,
                        HostFeedbackKind::Success => Color::Green,
                        HostFeedbackKind::Warning => Color::LightYellow,
                        HostFeedbackKind::Error => Color::Red,
                    };
                    (feedback.message.as_str(), color)
                })
                .unwrap_or((fallback, Color::Gray));
            frame.render_widget(
                Paragraph::new(message).style(Style::default().fg(color)),
                rows[6],
            );
        }
        HostModal::DockerForm(form) => {
            let rect = centered(area.width.saturating_sub(4).min(88), 12, area);
            frame.render_widget(Clear, rect);
            let shell = match form.shell {
                crate::config::DockerShell::Auto => "Auto (bash, then sh)",
                crate::config::DockerShell::Bash => "bash",
                crate::config::DockerShell::Sh => "sh",
            };
            let parent_style = if form.field == 0 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            let container_style = if form.field == 1 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            let shell_style = if form.field == 2 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            let running = if form.containers.is_empty() {
                "No running containers loaded; enter a name or press Ctrl+L to list".to_owned()
            } else {
                format!("Running containers: {}", form.containers.join(", "))
            };
            let content = vec![
                Line::from(vec![
                    Span::raw("SSH host: "),
                    Span::styled(format!("{} (↑/↓ choose)", form.ssh_host), parent_style),
                ]),
                Line::from(vec![
                    Span::raw("Container (↑/↓): "),
                    Span::styled(format!("{}█", form.container), container_style),
                ]),
                Line::from(vec![
                    Span::raw("Shell (↑/↓): "),
                    Span::styled(shell, shell_style),
                ]),
                Line::from(running),
                super::shortcut_line(&[("Tab", "field"), ("Ctrl+L", "list"), ("Ctrl+T", "test")]),
                super::shortcut_line(&[("Enter", "save"), ("Esc", "cancel")]),
            ];
            frame.render_widget(
                Paragraph::new(content).block(Block::default().borders(Borders::ALL).title(
                    if form.original.is_some() {
                        " Edit Docker Container Host "
                    } else {
                        " Add Docker Container Host "
                    },
                )),
                rect,
            );
            let test_status = if form.testing {
                "Testing connection…"
            } else if form.tested_revision == Some(form.revision) {
                "Connection succeeded! Test is optional"
            } else {
                "Connection test is optional"
            };
            let current_alias = crate::app::docker_alias(&DockerHostConfig {
                ssh_host: form.ssh_host.clone(),
                container: form.container.clone(),
                shell: form.shell,
            });
            let (test_status, test_color) = feedback
                .filter(|feedback| {
                    feedback
                        .alias
                        .as_deref()
                        .is_none_or(|alias| alias == current_alias || alias == form.ssh_host)
                })
                .map(|feedback| {
                    let color = match feedback.kind {
                        HostFeedbackKind::Running => Color::Yellow,
                        HostFeedbackKind::Success => Color::Green,
                        HostFeedbackKind::Warning => Color::LightYellow,
                        HostFeedbackKind::Error => Color::Red,
                    };
                    (feedback.message.as_str(), color)
                })
                .unwrap_or((
                    test_status,
                    if form.tested_revision == Some(form.revision) {
                        Color::Green
                    } else {
                        Color::Gray
                    },
                ));
            frame.render_widget(
                Paragraph::new(test_status).style(Style::default().fg(test_color)),
                Rect::new(
                    rect.x.saturating_add(1),
                    rect.bottom().saturating_sub(2),
                    rect.width.saturating_sub(2),
                    1,
                ),
            );
        }
        HostModal::ConfirmDelete {
            alias,
            docker_children,
            offset,
        } => {
            let rect = centered(
                area.width.saturating_sub(4).min(72),
                area.height.saturating_sub(4).clamp(7, 12),
                area,
            );
            frame.render_widget(Clear, rect);
            let visible = usize::from(rect.height.saturating_sub(6)).max(1);
            let children = docker_children
                .iter()
                .skip(*offset)
                .take(visible)
                .map(|child| Line::from(format!("• {child}")));
            let mut lines = vec![
                Line::from(format!("Delete host '{alias}'?")),
                Line::from("OpenSSH config and remote containers are not deleted."),
                Line::from(if docker_children.is_empty() {
                    "No configured Docker child hosts.".into()
                } else {
                    format!(
                        "{} configured Docker child host(s) will also be removed:",
                        docker_children.len()
                    )
                }),
            ];
            lines.extend(children);
            if docker_children.len() > visible {
                lines.push(Line::from(format!(
                    "↑/↓ scroll children · {}/{}",
                    offset + 1,
                    docker_children.len()
                )));
            }
            if let Some(feedback) = feedback
                && feedback
                    .alias
                    .as_deref()
                    .is_none_or(|feedback_alias| feedback_alias == alias)
            {
                let color = match feedback.kind {
                    HostFeedbackKind::Running => Color::Yellow,
                    HostFeedbackKind::Success => Color::Green,
                    HostFeedbackKind::Warning => Color::LightYellow,
                    HostFeedbackKind::Error => Color::Red,
                };
                lines.push(Line::styled(
                    feedback.message.as_str(),
                    Style::default().fg(color),
                ));
            }
            lines.push(Line::from("Enter confirm · Esc cancel"));
            frame.render_widget(
                Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Confirm Delete "),
                ),
                rect,
            );
        }
    }
}
