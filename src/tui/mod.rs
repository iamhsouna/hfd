pub mod app;
pub mod browser;
pub mod picker;
pub mod speedo;
pub mod ui;

use std::io::IsTerminal;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event, KeyEventKind};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::EventStream;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::progress::Phase;

pub use app::App;

/// Run the live dashboard until the user quits or the transfer finishes and
/// the user acknowledges it.
pub async fn run(mut app: App, cancel: CancellationToken, download: JoinHandle<()>) -> Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, cancel.clone(), download).await;
    ratatui::restore();
    result
}

async fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    cancel: CancellationToken,
    download: JoinHandle<()>,
) -> Result<()> {
    let mut reader = EventStream::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(120));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut download = Some(download);

    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        tokio::select! {
            maybe_event = reader.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => {
                        app.on_key(key, &cancel);
                    }
                    Some(Ok(Event::Resize(_, _))) => {}
                    Some(Err(_)) | None => {}
                    _ => {}
                }
            }
            _ = ticker.tick() => {}
        }

        if app.exit {
            break;
        }

        if let Some(handle) = &download
            && handle.is_finished()
        {
            download = None;
        }
    }

    if !app.is_terminal_phase() {
        cancel.cancel();
    }
    if let Some(handle) = download {
        let _ = handle.await;
    }

    let phase = app.hub.phase();
    if let Phase::Failed(reason) = phase {
        eprintln!("hfd: {reason}");
    }
    Ok(())
}

/// Whether the interactive dashboard should be used.
pub fn use_tui(requested: bool) -> bool {
    requested && std::io::stdout().is_terminal()
}
