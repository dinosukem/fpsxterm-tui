use super::*;

pub(super) fn render_transfers(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(65), Constraint::Min(3)])
        .split(area);
    let rows = app
        .transfers
        .iter()
        .map(|transfer| {
            let duration = if transfer.state == "Running" {
                transfer.elapsed.elapsed()
            } else {
                transfer.duration
            };
            let progress = transfer
                .total_bytes
                .map(|total| {
                    format!(
                        "{}%",
                        transfer
                            .bytes
                            .saturating_mul(100)
                            .checked_div(total.max(1))
                            .unwrap_or(0)
                            .min(100)
                    )
                })
                .unwrap_or_else(|| format!("{} B", transfer.bytes));
            Row::new(vec![
                Cell::from(format_vn(transfer.started_at)),
                Cell::from(format!("{}s", duration.as_secs())),
                Cell::from(transfer.source.clone()),
                Cell::from(transfer.destination.clone()),
                Cell::from(transfer.file_path.display().to_string()),
                Cell::from(format!("{} {progress}", transfer.state)),
            ])
        })
        .collect::<Vec<_>>();
    let header = Row::new([
        "Time (VN)",
        "Duration",
        "Source",
        "Destination",
        "File",
        "Status / progress",
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = TableState::default();
    if !app.transfers.is_empty() {
        state.select(Some(app.selected_transfer));
    }
    frame.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Length(20),
                Constraint::Length(10),
                Constraint::Percentage(22),
                Constraint::Percentage(22),
                Constraint::Percentage(27),
                Constraint::Percentage(15),
            ],
        )
        .header(header)
        .row_highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL).title(" Transfers ")),
        chunks[0],
        &mut state,
    );
    let details = app
        .transfers
        .get(app.selected_transfer)
        .map(|transfer| {
            format!(
                "File: {}\nSource: {}\nDestination: {}",
                transfer.file_path.display(),
                transfer.source,
                transfer.destination
            )
        })
        .unwrap_or_else(|| "No transfers yet. Start one with F6 in Files.".into());
    frame.render_widget(
        Paragraph::new(details).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Transfer details "),
        ),
        chunks[1],
    );
}
