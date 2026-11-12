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

mod context_menu;
mod dialogs;
mod files;
mod history;
mod hosts;
mod layout;
mod terminal;
mod transfers;

pub use layout::{file_context_menu_item_at, terminal_content_rect, workspace_tab_at};

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
        dialogs::render_host_modal(frame, modal, area);
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
    frame.render_widget(
        Paragraph::new(app.status.as_str()).style(Style::default().fg(Color::Gray)),
        footer[0],
    );
    let help = match app.screen {
        Screen::Hosts => &[
            ("Enter", "connect"),
            ("a", "add"),
            ("i", "import"),
            ("e", "edit"),
            ("Delete", "remove"),
            ("r", "refresh"),
        ][..],
        Screen::Terminal => &[
            ("Ctrl+]", "app focus"),
            ("n", "local shell"),
            ("Alt+1..9", "or Ctrl+1..9 sessions"),
            ("x", "close"),
        ][..],
        Screen::Files => &[
            ("Tab", "panels"),
            ("Enter", "open folder/edit file"),
            ("Ctrl+K", "or right-click actions"),
            ("F2", "rename"),
            ("Delete", "remove"),
            ("F6", "transfer"),
            ("Ctrl+L", "path"),
        ][..],
        Screen::Transfers => &[("↑/↓", "select"), ("c", "cancel selected transfer")][..],
        Screen::History => &[
            ("↑/↓", "select"),
            ("Enter", "reconnect"),
            ("Delete", "remove entry"),
        ][..],
    };
    frame.render_widget(Paragraph::new(shortcut_line(help)), footer[1]);
    frame.render_widget(
        Paragraph::new(shortcut_line(&[
            ("Ctrl+Tab", "recent"),
            ("Shift+Tab", "next"),
            ("Ctrl+Shift+Tab", "previous"),
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

fn render_screen(frame: &mut Frame<'_>, app: &App, area: Rect) {
    match app.screen {
        Screen::Hosts => hosts::render(frame, app, area),
        Screen::Terminal => terminal::render(frame, app, area),
        Screen::Files => files::render_files(frame, app, area),
        Screen::Transfers => transfers::render_transfers(frame, app, area),
        Screen::History => history::render_history(frame, app, area),
    }
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
