use ratatui::prelude::*;
use ratatui::widgets::*;

use super::app::App;
use crate::progress::{FileStatus, Phase};
use crate::util;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(frame, app, layout[0]);
    draw_body(frame, app, layout[1]);
    draw_footer(frame, app, layout[2]);

    if app.help {
        draw_help(frame, area, app);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    let repo = app
        .hub
        .files
        .first()
        .map(|file| file.repo.clone())
        .unwrap_or_default();
    let phase = app.hub.phase();
    let pause = if app.hub.is_paused() {
        "  [PAUSED]"
    } else {
        ""
    };
    let title = Line::from(vec![
        Span::styled(
            " hfd ",
            Style::new().fg(Color::Black).bg(Color::Cyan).bold(),
        ),
        Span::raw(" "),
        Span::styled(repo, Style::new().fg(Color::White).bold()),
        Span::styled(format!("  {}", phase_label(&phase)), phase_style(&phase)),
        Span::styled(pause, Style::new().fg(Color::Yellow).bold()),
    ]);
    frame.render_widget(Paragraph::new(title), rows[0]);

    let gauge = LineGauge::default()
        .ratio(app.hub.fraction())
        .label(format!("{:>3}%", app.hub.percent()))
        .filled_style(Style::new().fg(Color::Green).bold())
        .unfilled_style(Style::new().fg(Color::DarkGray));
    frame.render_widget(gauge, rows[1]);

    let stats = Line::from(vec![
        Span::styled(
            format!(" {} ", util::format_bytes(app.hub.downloaded())),
            Style::new().fg(Color::Cyan),
        ),
        Span::styled(
            format!("/ {} ", util::format_bytes(app.hub.total_bytes)),
            Style::new().fg(Color::DarkGray),
        ),
        Span::styled("• ", Style::new().fg(Color::DarkGray)),
        Span::styled(
            format!("{} ", util::format_speed(app.hub.speed() as f64)),
            Style::new().fg(Color::Green),
        ),
        Span::styled("• ", Style::new().fg(Color::DarkGray)),
        Span::styled(
            format!("ETA {} ", app.hub.eta()),
            Style::new().fg(Color::Yellow),
        ),
        Span::styled("• ", Style::new().fg(Color::DarkGray)),
        Span::styled(
            format!("active {} ", app.hub.active_count()),
            Style::new().fg(Color::Blue),
        ),
        Span::styled(
            format!("done {} ", app.hub.done_count()),
            Style::new().fg(Color::Green),
        ),
        Span::styled(
            format!("failed {} ", app.hub.failed_count()),
            Style::new().fg(if app.hub.failed_count() > 0 {
                Color::Red
            } else {
                Color::DarkGray
            }),
        ),
    ]);
    frame.render_widget(Paragraph::new(stats), rows[2]);
}

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
        .split(area);

    draw_files(frame, app, columns[0]);
    draw_details(frame, app, columns[1]);
}

fn draw_files(frame: &mut Frame, app: &mut App, area: Rect) {
    let widths = [
        Constraint::Length(3),
        Constraint::Min(24),
        Constraint::Length(16),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(8),
        Constraint::Length(13),
    ];

    let header = Row::new(vec![
        Cell::from("#"),
        Cell::from("file"),
        Cell::from("%"),
        Cell::from("size"),
        Cell::from("speed"),
        Cell::from("eta"),
        Cell::from("status"),
    ])
    .style(Style::new().fg(Color::Black).bg(Color::Cyan).bold());

    let rows: Vec<Row> = app
        .hub
        .files
        .iter()
        .map(|file| {
            let status = file.status();
            let percent = if file.total == 0 {
                format!("{:>6}", util::format_bytes(file.progress()))
            } else {
                format!("{} {:>3}%", bar(file.fraction(), 8), file.percent())
            };
            let speed = file.speed.load(std::sync::atomic::Ordering::Relaxed) as f64;
            let eta = if file.total > 0 {
                util::format_eta(file.total.saturating_sub(file.progress()), speed)
            } else {
                "--".to_string()
            };
            let name = util::truncate_middle(file.file_name(), 40);
            let row = Row::new(vec![
                Cell::from(format!("{}", file.index + 1)),
                Cell::from(name),
                Cell::from(percent),
                Cell::from(util::format_bytes(file.total)),
                Cell::from(util::format_speed(speed)),
                Cell::from(eta),
                Cell::from(status.label()),
            ]);
            row.style(status_style(&status))
        })
        .collect();

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::bordered()
                .title(format!(" files ({}) ", app.hub.files.len()))
                .border_style(Style::new().fg(Color::DarkGray)),
        )
        .row_highlight_style(Style::new().bg(Color::Indexed(236)).bold())
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn draw_details(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered()
        .title(" details ")
        .border_style(Style::new().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(5)])
        .split(inner);

    let Some(file) = app.hub.files.get(app.selected()) else {
        return;
    };
    let status = file.status();
    let speed = file.speed.load(std::sync::atomic::Ordering::Relaxed) as f64;
    let elapsed = file.elapsed().map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let average = if elapsed > 0.0 {
        file.progress() as f64 / elapsed
    } else {
        0.0
    };

    let lines = vec![
        info_line("repo", &file.repo, Color::White),
        info_line("path", &file.path, Color::White),
        info_line("dest", &util::display_path(&file.dest), Color::DarkGray),
        info_line("size", &util::format_bytes(file.total), Color::Cyan),
        info_line("done", &util::format_bytes(file.progress()), Color::Green),
        info_line(
            "speed",
            &format!(
                "{} (avg {})",
                util::format_speed(speed),
                util::format_speed(average)
            ),
            Color::Green,
        ),
        info_line("elapsed", &util::format_elapsed(elapsed), Color::Yellow),
        info_line("chunks", &format!("{}", file.chunks()), Color::Magenta),
        Line::from(vec![
            Span::styled("status  ", Style::new().fg(Color::DarkGray)),
            Span::styled(status.label(), status_style(&status)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), rows[0]);

    let history: Vec<u64> = file.history.lock().unwrap().iter().copied().collect();
    let sparkline = Sparkline::default()
        .block(
            Block::new()
                .title(" speed ")
                .title_style(Style::new().fg(Color::DarkGray)),
        )
        .data(history)
        .style(Style::new().fg(Color::Cyan));
    frame.render_widget(sparkline, rows[1]);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let message = app.hub.message();
    let line = if !message.is_empty() {
        Line::from(Span::styled(
            format!(" {message}"),
            Style::new().fg(Color::Yellow).bold(),
        ))
    } else {
        Line::from(vec![
            Span::styled(" q", key_style()),
            Span::raw(" quit  "),
            Span::styled("p", key_style()),
            Span::raw(" pause/resume  "),
            Span::styled("↑/↓", key_style()),
            Span::raw(" select  "),
            Span::styled("c", key_style()),
            Span::raw(" cancel  "),
            Span::styled("o", key_style()),
            Span::raw(" open folder  "),
            Span::styled("?", key_style()),
            Span::raw(" help"),
        ])
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    let popup = centered_rect(60, 60, area);
    frame.render_widget(Clear, popup);

    let lines = vec![
        Line::from(Span::styled(
            "hfd — keyboard",
            Style::new().fg(Color::Cyan).bold(),
        )),
        Line::from(""),
        help_line("q / Esc", "quit (saves resume state)"),
        help_line("p / Space", "pause or resume all transfers"),
        help_line("↑ / k", "select previous file"),
        help_line("↓ / j", "select next file"),
        help_line("g / G", "jump to first / last"),
        help_line("c", "cancel all downloads"),
        help_line("o", "open the output folder"),
        help_line("? / h", "toggle this help"),
        Line::from(""),
        Line::from(Span::styled("command", Style::new().fg(Color::DarkGray))),
        Line::from(Span::styled(
            format!("  {}", app.command),
            Style::new().fg(Color::DarkGray),
        )),
    ];
    let block = Block::bordered()
        .title(" help ")
        .border_style(Style::new().fg(Color::Cyan));
    frame.render_widget(
        Paragraph::new(lines).block(block.padding(Padding::horizontal(2))),
        popup,
    );
}

fn info_line(label: &str, value: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<8}"), Style::new().fg(Color::DarkGray)),
        Span::styled(value.to_string(), Style::new().fg(color)),
    ])
}

fn help_line(key: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {key:<12}"),
            Style::new().fg(Color::Yellow).bold(),
        ),
        Span::raw(description.to_string()),
    ])
}

fn key_style() -> Style {
    Style::new().fg(Color::Black).bg(Color::DarkGray).bold()
}

fn phase_label(phase: &Phase) -> &'static str {
    match phase {
        Phase::Preparing => "preparing",
        Phase::Downloading => "downloading",
        Phase::Paused => "paused",
        Phase::Done => "complete",
        Phase::Failed(_) => "finished with errors",
        Phase::Cancelled => "cancelled",
    }
}

fn phase_style(phase: &Phase) -> Style {
    match phase {
        Phase::Done => Style::new().fg(Color::Green).bold(),
        Phase::Failed(_) => Style::new().fg(Color::Red).bold(),
        Phase::Cancelled => Style::new().fg(Color::Yellow).bold(),
        Phase::Paused => Style::new().fg(Color::Yellow),
        _ => Style::new().fg(Color::Cyan),
    }
}

fn status_style(status: &FileStatus) -> Style {
    match status {
        FileStatus::Done => Style::new().fg(Color::Green),
        FileStatus::Skipped => Style::new().fg(Color::DarkGray),
        FileStatus::Downloading => Style::new().fg(Color::Cyan),
        FileStatus::Verifying => Style::new().fg(Color::Magenta),
        FileStatus::Paused => Style::new().fg(Color::Yellow),
        FileStatus::Pending => Style::new().fg(Color::Gray),
        FileStatus::Failed(_) => Style::new().fg(Color::Red),
        FileStatus::Cancelled => Style::new().fg(Color::Yellow),
    }
}

fn bar(fraction: f64, width: usize) -> String {
    let filled = (fraction * width as f64).round() as usize;
    let mut out = String::with_capacity(width);
    for index in 0..width {
        out.push(if index < filled { '█' } else { '░' });
    }
    out
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
