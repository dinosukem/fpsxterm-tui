use super::*;

pub(super) fn render(frame: &mut Frame<'_>, choice: usize, area: Rect) {
    let rect = super::centered(48, 12, area);
    frame.render_widget(Clear, rect);
    let items = [
        "View / open",
        "Edit with configured terminal editor",
        "Transfer",
        "Rename",
        "New folder",
        "Delete",
        "Refresh",
    ];
    let lines = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let label = if index == choice {
                format!("› {item}")
            } else {
                format!("  {item}")
            };
            let style = if index == choice {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(Span::styled(label, style))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" File actions "),
        ),
        rect,
    );
}
