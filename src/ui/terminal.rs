use super::*;

pub(super) fn render(frame: &mut Frame<'_>, app: &App, area: Rect) {
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
