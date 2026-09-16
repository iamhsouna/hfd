use anyhow::Result;
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind, read};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::api::{ram_estimate, stars};
use crate::util;

pub struct PickItem {
    pub path: String,
    pub quant: Option<String>,
    pub size: u64,
    pub selected: bool,
}

impl PickItem {
    pub fn new(path: String, quant: Option<String>, size: u64) -> Self {
        Self {
            path,
            quant,
            size,
            selected: false,
        }
    }

    fn label(&self) -> String {
        self.quant.clone().unwrap_or_else(|| "—".to_string())
    }
}

/// Show the interactive quantization picker, returning selected indices or
/// `None` when the user cancels.
pub fn pick(items: &mut [PickItem], title: &str) -> Result<Option<Vec<usize>>> {
    if items.is_empty() {
        return Ok(None);
    }
    let mut terminal = ratatui::init();
    let mut state = TableState::default();
    state.select(Some(0));
    let result = pick_loop(&mut terminal, items, &mut state, title);
    ratatui::restore();
    result
}

fn pick_loop(
    terminal: &mut DefaultTerminal,
    items: &mut [PickItem],
    state: &mut TableState,
    title: &str,
) -> Result<Option<Vec<usize>>> {
    loop {
        terminal.draw(|frame| draw_picker(frame, items, state, title))?;
        let event = read()?;
        let Event::Key(key) = event else { continue };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        let cursor = state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
            KeyCode::Enter => {
                let selected: Vec<usize> = items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.selected)
                    .map(|(index, _)| index)
                    .collect();
                return Ok(Some(selected));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let next = (cursor + 1).min(items.len() - 1);
                state.select(Some(next));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.select(Some(cursor.saturating_sub(1)));
            }
            KeyCode::Char(' ') => {
                items[cursor].selected = !items[cursor].selected;
            }
            KeyCode::Char('a') => {
                let all = items.iter().all(|item| item.selected);
                for item in items.iter_mut() {
                    item.selected = !all;
                }
            }
            KeyCode::Char('n') => {
                for item in items.iter_mut() {
                    item.selected = false;
                }
            }
            _ => {}
        }
    }
}

fn draw_picker(frame: &mut Frame, items: &[PickItem], state: &mut TableState, title: &str) {
    let area = frame.area();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

    let selected_count = items.iter().filter(|item| item.selected).count();
    let selected_bytes: u64 = items
        .iter()
        .filter(|item| item.selected)
        .map(|item| item.size)
        .sum();
    let header = Line::from(vec![
        Span::styled(
            " hfd analyze ",
            Style::new().fg(Color::Black).bg(Color::Magenta).bold(),
        ),
        Span::raw(" "),
        Span::styled(title.to_string(), Style::new().fg(Color::White).bold()),
        Span::raw("  "),
        Span::styled(
            format!(
                "{selected_count} selected · {}",
                util::format_bytes(selected_bytes)
            ),
            Style::new().fg(Color::Green),
        ),
    ]);
    frame.render_widget(Paragraph::new(header), layout[0]);

    let widths = [
        Constraint::Length(3),
        Constraint::Length(10),
        Constraint::Length(7),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Min(20),
    ];
    let header_row = Row::new(vec![
        Cell::from(""),
        Cell::from("quant"),
        Cell::from("quality"),
        Cell::from("size"),
        Cell::from("RAM (est)"),
        Cell::from("file"),
    ])
    .style(Style::new().fg(Color::Black).bg(Color::Magenta).bold());

    let rows: Vec<Row> = items
        .iter()
        .map(|item| {
            let mark = if item.selected { "[x]" } else { "[ ]" };
            let quality = item
                .quant
                .as_deref()
                .map(stars)
                .unwrap_or_else(|| "     ".to_string());
            let mut name = item.path.clone();
            if item
                .quant
                .as_deref()
                .map(crate::api::filter::is_recommended)
                .unwrap_or(false)
            {
                name.push_str("  ★ recommended");
            }
            let style = if item.selected {
                Style::new().fg(Color::Green)
            } else {
                Style::new()
            };
            Row::new(vec![
                Cell::from(mark),
                Cell::from(item.label()),
                Cell::from(quality),
                Cell::from(util::format_bytes(item.size)),
                Cell::from(format!("~{}", util::format_bytes(ram_estimate(item.size)))),
                Cell::from(name),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(rows, widths)
        .header(header_row)
        .block(
            Block::bordered()
                .title(" GGUF quantizations ")
                .border_style(Style::new().fg(Color::Magenta)),
        )
        .row_highlight_style(Style::new().bg(Color::Indexed(236)).bold())
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, layout[1], state);

    let footer = Line::from(vec![
        Span::styled(" space", key_style()),
        Span::raw(" toggle  "),
        Span::styled("a", key_style()),
        Span::raw(" all  "),
        Span::styled("n", key_style()),
        Span::raw(" none  "),
        Span::styled("enter", key_style()),
        Span::raw(" download  "),
        Span::styled("q", key_style()),
        Span::raw(" cancel"),
    ]);
    frame.render_widget(Paragraph::new(footer), layout[2]);
}

fn key_style() -> Style {
    Style::new().fg(Color::Black).bg(Color::DarkGray).bold()
}

use ratatui::DefaultTerminal;
