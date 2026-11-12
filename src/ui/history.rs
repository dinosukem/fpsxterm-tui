use super::*;

pub(super) fn render_history(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let items = if app.history.is_empty() {
        vec![ListItem::new("No SSH connections yet.")]
    } else {
        app.history
            .iter()
            .map(|entry| {
                ListItem::new(format!(
                    "{}  {}  {}",
                    format_unix_vn(entry.at_unix),
                    entry.alias,
                    entry.status
                ))
            })
            .collect()
    };
    let mut state = ListState::default();
    if !app.history.is_empty() {
        state.select(Some(app.selected_history));
    }
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(area);
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" SSH History · newest first "),
            )
            .highlight_symbol("› ")
            .highlight_style(Style::default().fg(Color::Yellow)),
        columns[0],
        &mut state,
    );
    let details = app
        .history
        .get(app.selected_history)
        .map(|entry| {
            format!(
                "Host: {}\nTime: {}\nStatus: {}\n\nEnter reconnect\nDelete remove",
                entry.alias,
                format_unix_vn(entry.at_unix),
                entry.status
            )
        })
        .unwrap_or_else(|| {
            "History stores host alias and result only; credentials are not saved.".into()
        });
    frame.render_widget(
        Paragraph::new(details).block(Block::default().borders(Borders::ALL).title(" Details ")),
        columns[1],
    );
}
