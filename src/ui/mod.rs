use {
    crate::{
        app::{
            App,
            FileInputOperation,
            FileModal,
            HostFeedback,
            HostFeedbackKind,
            HostModal,
            Screen,
        },
        config::DockerHostConfig,
    },
    ratatui::{
        Frame,
        layout::{
            Constraint,
            Direction,
            Layout,
            Margin,
            Rect,
        },
        style::{
            Color,
            Modifier,
            Style,
        },
        text::{
            Line,
            Span,
        },
        widgets::{
            Block,
            Borders,
            Cell,
            Clear,
            List,
            ListItem,
            ListState,
            Paragraph,
            Row,
            Table,
            TableState,
            Tabs,
        },
    },
};

mod context_menu;
mod dialogs;
mod files;
mod history;
mod hosts;
mod layout;
mod terminal;
mod transfers;

pub use layout::{
    file_context_menu_item_at,
    terminal_content_rect,
    workspace_tab_at,
};

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
        dialogs::render_host_modal(frame, modal, app.host_feedback.as_ref(), area);
    }
    if let Some(modal) = &app.file_modal {
        dialogs::render_file_modal(frame, modal, area);
    }
    let footer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(chunks[3]);
    let (status, status_color) = status_presentation(&app.status);
    frame.render_widget(
        Paragraph::new(status).style(Style::default().fg(status_color)),
        footer[0],
    );
    let help = match app.screen {
        Screen::Hosts => {
            &[
                ("Enter", "connect"),
                ("T", "test connection"),
                ("A", "add"),
                ("D", "add Docker"),
                ("L", "list containers"),
                ("I", "import"),
                ("E", "edit"),
                ("Delete", "remove"),
                ("R", "refresh"),
            ][..]
        }
        Screen::Terminal => {
            &[
                ("Ctrl+]", "app focus"),
                ("N", "local shell"),
                ("Alt+1..9", "or Ctrl+1..9 sessions"),
                ("Ctrl+T", "test selected host"),
                ("X", "close"),
            ][..]
        }
        Screen::Files => {
            &[
                ("Tab", "panels"),
                ("Enter", "open folder/edit file"),
                ("Ctrl+K", "or right-click actions"),
                ("F2", "rename"),
                ("Delete", "remove"),
                ("F6", "transfer"),
                ("Ctrl+L", "path"),
            ][..]
        }
        Screen::Transfers => &[("↑/↓", "select"), ("C", "cancel selected transfer")][..],
        Screen::History => {
            &[
                ("↑/↓", "select"),
                ("Enter", "reconnect"),
                ("Delete", "remove entry"),
            ][..]
        }
        Screen::Configuration => {
            &[
                ("↑/↓", "select"),
                ("Space", "toggle"),
                ("Enter", "edit"),
                ("Ctrl+A", "clear value"),
                ("Ctrl+S", "save"),
            ][..]
        }
    };
    frame.render_widget(Paragraph::new(shortcut_line(help)), footer[1]);
    frame.render_widget(
        Paragraph::new(shortcut_line(&[
            ("Alt+H/T/F", "Hosts/Terminal/Files"),
            ("Alt+Shift+H/T", "History/Transfers"),
            ("Alt+C", "Configuration"),
            ("Alt+←/→", "previous/next"),
            ("F8", "recent"),
            ("Ctrl+Q", "quit"),
        ])),
        footer[2],
    );
}

fn shortcut_line(items: &[(&str, &str)]) -> Line<'static> {
    let key_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Gray)
        .add_modifier(Modifier::BOLD);
    let mut spans = Vec::with_capacity(items.len() * 2);
    for (index, (key, action)) in items.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(format!(" {key} "), key_style));
        spans.push(Span::raw(format!(" {action}")));
    }
    Line::from(spans)
}

fn status_presentation(status: &str) -> (String, Color) {
    let lower = status.to_ascii_lowercase();
    if [
        "could not",
        "failed",
        "error",
        "invalid",
        "missing tools",
        "cannot",
        "not found",
        "timed out",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        (format!("❌ {status}"), Color::Red)
    } else if status.contains("Connection succeeded:")
        || (status.starts_with("Connection to ") && status.contains(" succeeded:"))
    {
        (status.to_owned(), Color::Green)
    } else {
        (status.to_owned(), Color::Gray)
    }
}

fn render_screen(frame: &mut Frame<'_>, app: &App, area: Rect) {
    match app.screen {
        Screen::Hosts => hosts::render(frame, app, area),
        Screen::Terminal => terminal::render(frame, app, area),
        Screen::Files => files::render_files(frame, app, area),
        Screen::Transfers => transfers::render_transfers(frame, app, area),
        Screen::History => history::render_history(frame, app, area),
        Screen::Configuration => render_configuration(frame, app, area),
    }
}

fn render_configuration(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let entries = [
        ("Theme", app.configuration_draft.ui.theme.clone()),
        (
            "Mouse support",
            app.configuration_draft.ui.mouse_enabled.to_string(),
        ),
        (
            "Show hidden files",
            app.configuration_draft.ui.show_hidden_files.to_string(),
        ),
        (
            "Confirm delete",
            app.configuration_draft.files.confirm_delete.to_string(),
        ),
        (
            "Confirm overwrite",
            app.configuration_draft.files.confirm_overwrite.to_string(),
        ),
        (
            "Local shell",
            app.configuration_draft.terminal.shell.display().to_string(),
        ),
        (
            "Editor command",
            app.configuration_draft.terminal.editor.join(" "),
        ),
        (
            "Scrollback lines",
            app.configuration_draft
                .terminal
                .scrollback_lines
                .to_string(),
        ),
        (
            "Concurrent transfers",
            app.configuration_draft.transfer.max_concurrent.to_string(),
        ),
    ];
    let rows = entries
        .iter()
        .enumerate()
        .map(|(i, (name, value))| {
            let prefix = if i == app.configuration_field {
                "› "
            } else {
                "  "
            };
            let value = if app.configuration_editing && i == app.configuration_field {
                format!("{value}█")
            } else {
                value.clone()
            };
            let style = if app.configuration_editing && i == app.configuration_field {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if i == app.configuration_field {
                Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else if app.configuration_hover == Some(i) {
                Style::default().fg(Color::Black).bg(Color::Gray)
            } else {
                Style::default()
            };
            Line::from(format!("{prefix}{name:<24} {value}")).style(style)
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(rows)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Configuration · Ctrl+S save · Esc discard "),
            )
            .scroll((app.configuration_scroll as u16, 0)),
        area,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_red_with_cross_while_connection_success_stays_green() {
        let (error, error_color) =
            status_presentation("Could not read remote directory: missing tools");
        assert!(error.starts_with("❌ "));
        assert_eq!(error_color, Color::Red);

        let (success, success_color) =
            status_presentation("Connection succeeded: SSH authentication succeeded");
        assert!(!success.starts_with("❌ "));
        assert_eq!(success_color, Color::Green);
    }
}
