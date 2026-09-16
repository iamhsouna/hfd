use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::TableState;
use tokio_util::sync::CancellationToken;

use crate::progress::{Phase, ProgressHub};

pub struct App {
    pub hub: Arc<ProgressHub>,
    pub output_root: PathBuf,
    pub table_state: TableState,
    pub help: bool,
    pub confirm_quit: bool,
    pub exit: bool,
    pub command: String,
}

impl App {
    pub fn new(hub: Arc<ProgressHub>, output_root: PathBuf, command: String) -> Self {
        let mut table_state = TableState::default();
        if !hub.files.is_empty() {
            table_state.select(Some(0));
        }
        Self {
            hub,
            output_root,
            table_state,
            help: false,
            confirm_quit: false,
            exit: false,
            command,
        }
    }

    pub fn selected(&self) -> usize {
        self.table_state.selected().unwrap_or(0)
    }

    pub fn is_terminal_phase(&self) -> bool {
        matches!(
            self.hub.phase(),
            Phase::Done | Phase::Failed(_) | Phase::Cancelled
        )
    }

    pub fn on_key(&mut self, key: KeyEvent, cancel: &CancellationToken) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.is_terminal_phase() || self.hub.active_count() == 0 || self.confirm_quit {
                    self.exit = true;
                } else {
                    self.confirm_quit = true;
                    self.hub.set_message(
                        "Press q again to quit — progress is saved and resumes next run",
                    );
                }
            }
            KeyCode::Char('p') | KeyCode::Char(' ') => {
                self.confirm_quit = false;
                self.hub.toggle_pause();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.hub.files.len();
                if len > 0 {
                    let next = (self.selected() + 1).min(len - 1);
                    self.table_state.select(Some(next));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let prev = self.selected().saturating_sub(1);
                self.table_state.select(Some(prev));
            }
            KeyCode::Char('g') => {
                if !self.hub.files.is_empty() {
                    self.table_state.select(Some(0));
                }
            }
            KeyCode::Char('G') => {
                if !self.hub.files.is_empty() {
                    self.table_state.select(Some(self.hub.files.len() - 1));
                }
            }
            KeyCode::Char('o') => {
                open_folder(&self.output_root);
            }
            KeyCode::Char('c') => {
                self.hub
                    .set_message("Cancelling downloads (progress saved)…");
                cancel.cancel();
            }
            KeyCode::Char('?') | KeyCode::Char('h') => {
                self.help = !self.help;
            }
            _ => {}
        }
    }
}

fn open_folder(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";

    let _ = std::process::Command::new(program).arg(path).spawn();
}
