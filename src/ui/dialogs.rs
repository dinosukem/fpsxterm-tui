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
        FileModal::ContextMenu { choice, .. } => context_menu::render(frame, *choice, area),
    }
}
pub(super) fn render_host_modal(frame: &mut Frame<'_>, modal: &HostModal, area: Rect) {
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
