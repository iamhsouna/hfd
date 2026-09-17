use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::EventStream;
use ratatui::prelude::*;
use ratatui::widgets::*;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::api::repo::SearchHit;
use crate::api::{HfClient, RemoteFile, RepoInfo, ram_estimate, stars};
use crate::commands::Settings;
use crate::download::{self, DownloadPlan, RunConfig};
use crate::progress::{self, FileProgress, ProgressHub};
use crate::store::{self, Manifest};
use crate::tui::app::App;
use crate::tui::ui;
use crate::util;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    Search,
    Files,
    Download,
    Local,
    LocalDetail,
}

struct FileItem {
    file: RemoteFile,
    quant: Option<String>,
    selected: bool,
}

struct Session {
    app: App,
    token: CancellationToken,
    handle: Option<JoinHandle<()>>,
    sampler: JoinHandle<()>,
}

enum Outcome {
    None,
    Quit,
    Search(String),
    OpenRepo(String),
}

pub struct Browser {
    client: Arc<HfClient>,
    settings: Settings,
    screen: Screen,
    quit: bool,
    status: String,
    dataset: bool,
    input: String,
    search_focus: SearchFocus,
    results: Vec<SearchHit>,
    result_state: TableState,
    repo: Option<RepoInfo>,
    files: Vec<FileItem>,
    file_state: TableState,
    local: Vec<(PathBuf, Manifest)>,
    local_state: TableState,
    local_detail: Option<(PathBuf, Manifest)>,
    session: Option<Session>,
    menu_state: TableState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchFocus {
    Input,
    Results,
}

impl Browser {
    pub fn new(client: Arc<HfClient>, settings: Settings) -> Self {
        let mut menu_state = TableState::default();
        menu_state.select(Some(0));
        let mut local_state = TableState::default();
        local_state.select(Some(0));
        let local = store::find_manifests(&settings.output_root);
        Self {
            client,
            settings,
            screen: Screen::Home,
            quit: false,
            status: String::new(),
            dataset: false,
            input: String::new(),
            search_focus: SearchFocus::Input,
            results: Vec::new(),
            result_state: TableState::default(),
            repo: None,
            files: Vec::new(),
            file_state: TableState::default(),
            local,
            local_state,
            local_detail: None,
            session: None,
            menu_state,
        }
    }

    // ------------------------------------------------------------- key ----

    fn on_key(&mut self, key: KeyEvent) -> Outcome {
        match self.screen {
            Screen::Home => self.on_key_home(key),
            Screen::Search => self.on_key_search(key),
            Screen::Files => self.on_key_files(key),
            Screen::Download => self.on_key_download(key),
            Screen::Local => self.on_key_local(key),
            Screen::LocalDetail => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter) {
                    self.screen = Screen::Local;
                }
                Outcome::None
            }
        }
    }

    fn on_key_home(&mut self, key: KeyEvent) -> Outcome {
        let len = 3;
        let current = self.menu_state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Outcome::Quit,
            KeyCode::Down | KeyCode::Char('j') => {
                self.menu_state.select(Some((current + 1).min(len - 1)));
                Outcome::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.menu_state.select(Some(current.saturating_sub(1)));
                Outcome::None
            }
            KeyCode::Enter => match current {
                0 => {
                    self.screen = Screen::Search;
                    self.search_focus = SearchFocus::Input;
                    Outcome::None
                }
                1 => {
                    self.refresh_local();
                    self.screen = Screen::Local;
                    Outcome::None
                }
                _ => Outcome::Quit,
            },
            _ => Outcome::None,
        }
    }

    fn on_key_search(&mut self, key: KeyEvent) -> Outcome {
        match self.search_focus {
            SearchFocus::Input => match key.code {
                KeyCode::Esc => {
                    self.screen = Screen::Home;
                    Outcome::None
                }
                KeyCode::Enter => Outcome::Search(self.input.trim().to_string()),
                KeyCode::Backspace => {
                    self.input.pop();
                    Outcome::None
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.input.clear();
                    Outcome::None
                }
                KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.dataset = !self.dataset;
                    Outcome::None
                }
                KeyCode::Down | KeyCode::Tab => {
                    if !self.results.is_empty() {
                        self.search_focus = SearchFocus::Results;
                        if self.result_state.selected().is_none() {
                            self.result_state.select(Some(0));
                        }
                    }
                    Outcome::None
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.input.push(c);
                    Outcome::None
                }
                _ => Outcome::None,
            },
            SearchFocus::Results => match key.code {
                KeyCode::Esc => {
                    self.search_focus = SearchFocus::Input;
                    Outcome::None
                }
                KeyCode::Up => {
                    let current = self.result_state.selected().unwrap_or(0);
                    self.result_state.select(Some(current.saturating_sub(1)));
                    Outcome::None
                }
                KeyCode::Down => {
                    let current = self.result_state.selected().unwrap_or(0);
                    let next = (current + 1).min(self.results.len().saturating_sub(1));
                    self.result_state.select(Some(next));
                    Outcome::None
                }
                KeyCode::Enter => match self.result_state.selected() {
                    Some(index) if index < self.results.len() => {
                        Outcome::OpenRepo(self.results[index].id.clone())
                    }
                    _ => Outcome::None,
                },
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.search_focus = SearchFocus::Input;
                    self.input.push(c);
                    Outcome::None
                }
                _ => Outcome::None,
            },
        }
    }

    fn on_key_files(&mut self, key: KeyEvent) -> Outcome {
        let len = self.files.len();
        if len == 0 {
            if key.code == KeyCode::Esc {
                self.screen = Screen::Search;
            }
            return Outcome::None;
        }
        let current = self.file_state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Esc => {
                self.screen = Screen::Search;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.file_state.select(Some(current.saturating_sub(1)));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.file_state.select(Some((current + 1).min(len - 1)));
            }
            KeyCode::Char(' ') => {
                self.files[current].selected = !self.files[current].selected;
            }
            KeyCode::Char('a') => {
                let all = self.files.iter().all(|item| item.selected);
                for item in self.files.iter_mut() {
                    item.selected = !all;
                }
            }
            KeyCode::Char('n') => {
                for item in self.files.iter_mut() {
                    item.selected = false;
                }
            }
            KeyCode::Char('o') => {
                open_folder(&self.settings.output_root);
            }
            KeyCode::Char('g') => self.file_state.select(Some(0)),
            KeyCode::Char('G') => self.file_state.select(Some(len - 1)),
            KeyCode::Enter => {
                self.start_download();
            }
            _ => {}
        }
        Outcome::None
    }

    fn on_key_download(&mut self, key: KeyEvent) -> Outcome {
        let Some(session) = self.session.as_mut() else {
            self.screen = Screen::Home;
            return Outcome::None;
        };
        session.app.on_key(key, &session.token.clone());
        if session.app.exit {
            self.finish_session();
            self.screen = if self.repo.is_some() {
                Screen::Files
            } else {
                Screen::Home
            };
        }
        Outcome::None
    }

    fn on_key_local(&mut self, key: KeyEvent) -> Outcome {
        let len = self.local.len();
        let current = self.local_state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Esc => self.screen = Screen::Home,
            KeyCode::Up | KeyCode::Char('k') => {
                self.local_state.select(Some(current.saturating_sub(1)));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if len > 0 {
                    self.local_state.select(Some((current + 1).min(len - 1)));
                }
            }
            KeyCode::Char('o') => open_folder(&self.settings.output_root),
            KeyCode::Enter => {
                if let Some(entry) = self.local.get(current) {
                    self.local_detail = Some(entry.clone());
                    self.screen = Screen::LocalDetail;
                }
            }
            _ => {}
        }
        Outcome::None
    }

    // --------------------------------------------------------- actions ----

    async fn do_search(&mut self, query: &str) {
        if query.is_empty() {
            self.status = "Type a search query, or org/name to open a repo".into();
            return;
        }
        if query.contains('/') && !query.contains(' ') {
            self.do_open_repo(query).await;
            return;
        }
        let repo_type = if self.dataset { "datasets" } else { "models" };
        self.status = format!("searching {repo_type} for “{query}”…");
        let request = self.client.clone();
        let query = query.to_string();
        match request.search(repo_type, &query, 50).await {
            Ok(hits) => {
                self.status = format!("{} results", hits.len());
                self.results = hits;
                self.result_state.select(if self.results.is_empty() {
                    None
                } else {
                    Some(0)
                });
                self.search_focus = if self.results.is_empty() {
                    SearchFocus::Input
                } else {
                    SearchFocus::Results
                };
            }
            Err(error) => self.status = format!("search failed: {error}"),
        }
    }

    async fn do_open_repo(&mut self, repo: &str) {
        let repo_type = if self.dataset { "datasets" } else { "models" };
        self.status = format!("loading {repo}…");
        let client = self.client.clone();
        let revision = self.settings.revision.clone();
        match client.tree(repo_type, repo, &revision).await {
            Ok(info) => {
                self.status = format!(
                    "{} · {} files · {}",
                    info.id,
                    info.files.len(),
                    util::format_bytes(info.total_size())
                );
                self.files = info
                    .files
                    .iter()
                    .map(|file| FileItem {
                        quant: crate::api::gguf_quant(&file.path),
                        file: file.clone(),
                        selected: false,
                    })
                    .collect();
                self.repo = Some(info);
                self.file_state.select(Some(0));
                self.screen = Screen::Files;
            }
            Err(error) => self.status = format!("could not open {repo}: {error}"),
        }
    }

    fn start_download(&mut self) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let current = self.file_state.selected().unwrap_or(0);
        let mut selected: Vec<RemoteFile> = self
            .files
            .iter()
            .filter(|item| item.selected)
            .map(|item| item.file.clone())
            .collect();
        if selected.is_empty()
            && let Some(item) = self.files.get(current)
        {
            selected.push(item.file.clone());
        }
        if selected.is_empty() {
            self.status = "Nothing to download".into();
            return;
        }

        let repo_path = store::repo_dir(&self.settings.output_root, &repo.id);
        let hub_files: Vec<Arc<FileProgress>> = selected
            .iter()
            .enumerate()
            .map(|(index, file)| {
                Arc::new(FileProgress::new(
                    index,
                    repo.id.clone(),
                    file.path.clone(),
                    repo_path.join(&file.path),
                    file.size,
                ))
            })
            .collect();
        let hub = Arc::new(ProgressHub::new(hub_files));
        let token = CancellationToken::new();
        let sampler = progress::spawn_sampler(hub.clone(), token.clone());
        let plan = DownloadPlan {
            repo: repo.clone(),
            files: selected.clone(),
            output_root: self.settings.output_root.clone(),
            command: format!("hfd tui  # {}", repo.id),
        };
        let run_config = RunConfig {
            connections: self.settings.connections,
            max_active: self.settings.max_active,
            verify: self.settings.verify,
            force: false,
        };
        let client = self.client.clone();
        let hub_task = hub.clone();
        let token_task = token.clone();
        let handle = tokio::spawn(async move {
            download::run(client, plan, hub_task, token_task, run_config).await;
        });
        let app = App::new(
            hub.clone(),
            self.settings.output_root.clone(),
            format!("hfd tui · {}", repo.id),
        );
        self.session = Some(Session {
            app,
            token,
            handle: Some(handle),
            sampler,
        });
        self.screen = Screen::Download;
    }

    fn finish_session(&mut self) {
        if let Some(mut session) = self.session.take() {
            session.token.cancel();
            session.sampler.abort();
            if let Some(handle) = session.handle.take() {
                handle.abort();
            }
        }
        self.refresh_local();
    }

    fn poll_session(&mut self) {
        let finished = self
            .session
            .as_ref()
            .and_then(|session| session.handle.as_ref())
            .map(|handle| handle.is_finished())
            .unwrap_or(false);
        if finished && let Some(session) = self.session.as_mut() {
            session.handle = None;
        }
    }

    fn refresh_local(&mut self) {
        self.local = store::find_manifests(&self.settings.output_root);
        if self.local.is_empty() {
            self.local_state.select(None);
        } else {
            let index = self
                .local_state
                .selected()
                .unwrap_or(0)
                .min(self.local.len() - 1);
            self.local_state.select(Some(index));
        }
    }

    // ---------------------------------------------------------- drawing ----

    fn draw(&mut self, frame: &mut Frame) {
        if self.screen == Screen::Download
            && let Some(session) = self.session.as_mut()
        {
            ui::draw(frame, &mut session.app);
            return;
        }

        let area = frame.area();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(4),
                Constraint::Length(1),
            ])
            .split(area);

        let title = match self.screen {
            Screen::Home => "hfd · home".to_string(),
            Screen::Search => format!(
                "hfd · {} search",
                if self.dataset { "dataset" } else { "model" }
            ),
            Screen::Files => format!(
                "hfd · {}",
                self.repo.as_ref().map(|r| r.id.clone()).unwrap_or_default()
            ),
            Screen::Local => format!(
                "hfd · local · {}",
                util::display_path(&self.settings.output_root)
            ),
            Screen::LocalDetail => "hfd · local · details".to_string(),
            Screen::Download => String::new(),
        };
        let header = Line::from(vec![
            Span::styled(
                " hfd ",
                Style::new().fg(Color::Black).bg(Color::Cyan).bold(),
            ),
            Span::raw(" "),
            Span::styled(title, Style::new().fg(Color::White).bold()),
            Span::raw("  "),
            Span::styled(self.status.clone(), Style::new().fg(Color::Yellow)),
        ]);
        frame.render_widget(Paragraph::new(header), layout[0]);

        match self.screen {
            Screen::Home => self.draw_home(frame, layout[1]),
            Screen::Search => self.draw_search(frame, layout[1]),
            Screen::Files => self.draw_files(frame, layout[1]),
            Screen::Local => self.draw_local(frame, layout[1]),
            Screen::LocalDetail => self.draw_local_detail(frame, layout[1]),
            Screen::Download => {}
        }

        let footer = match self.screen {
            Screen::Home => " ↑↓ move   enter select   q quit",
            Screen::Search => match self.search_focus {
                SearchFocus::Input => {
                    " type query or org/name   enter search/open   ctrl-t model/dataset   esc back"
                }
                SearchFocus::Results => " ↑↓ move   enter open   esc back to input",
            },
            Screen::Files => {
                " space select   a all   n none   enter download   o open folder   esc back"
            }
            Screen::Local => " ↑↓ move   enter details   o open folder   esc back",
            Screen::LocalDetail => " esc back",
            Screen::Download => "",
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                footer,
                Style::new().fg(Color::Gray),
            ))),
            layout[2],
        );
    }

    fn draw_home(&mut self, frame: &mut Frame, area: Rect) {
        let items = [
            "Search HuggingFace and download",
            "Browse local downloads",
            "Quit",
        ];
        let rows: Vec<Row> = items
            .iter()
            .map(|item| Row::new(vec![Cell::from(*item)]))
            .collect();
        let table = Table::new(rows, [Constraint::Min(30)])
            .block(
                Block::bordered()
                    .title(" menu ")
                    .border_style(Style::new().fg(Color::Cyan)),
            )
            .row_highlight_style(Style::new().bg(Color::Indexed(236)).bold())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(table, area, &mut self.menu_state);
    }

    fn draw_search(&mut self, frame: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(3)])
            .split(area);

        let input_style = if self.search_focus == SearchFocus::Input {
            Style::new().fg(Color::Cyan).bold()
        } else {
            Style::new().fg(Color::Gray)
        };
        let cursor = if self.search_focus == SearchFocus::Input {
            "█"
        } else {
            ""
        };
        let input = Paragraph::new(Line::from(vec![
            Span::styled("query ", Style::new().fg(Color::DarkGray)),
            Span::styled(format!("{}{}", self.input, cursor), input_style),
        ]))
        .block(
            Block::bordered()
                .title(format!(
                    " search {} ",
                    if self.dataset { "datasets" } else { "models" }
                ))
                .border_style(Style::new().fg(if self.search_focus == SearchFocus::Input {
                    Color::Cyan
                } else {
                    Color::DarkGray
                })),
        );
        frame.render_widget(input, rows[0]);

        let header = Row::new(vec![
            Cell::from("repository"),
            Cell::from("downloads"),
            Cell::from("likes"),
            Cell::from("pipeline"),
        ])
        .style(Style::new().fg(Color::Black).bg(Color::Cyan).bold());
        let rows_data: Vec<Row> = self
            .results
            .iter()
            .map(|hit| {
                Row::new(vec![
                    Cell::from(hit.id.clone()),
                    Cell::from(hit.downloads.to_string()),
                    Cell::from(hit.likes.to_string()),
                    Cell::from(hit.pipeline_tag.clone().unwrap_or_default()),
                ])
            })
            .collect();
        let table = Table::new(
            rows_data,
            [
                Constraint::Min(30),
                Constraint::Length(12),
                Constraint::Length(8),
                Constraint::Length(20),
            ],
        )
        .header(header)
        .block(
            Block::bordered()
                .title(" results ")
                .border_style(
                    Style::new().fg(if self.search_focus == SearchFocus::Results {
                        Color::Cyan
                    } else {
                        Color::DarkGray
                    }),
                ),
        )
        .row_highlight_style(Style::new().bg(Color::Indexed(236)).bold())
        .highlight_symbol("▶ ");
        frame.render_stateful_widget(table, rows[1], &mut self.result_state);
    }

    fn draw_files(&mut self, frame: &mut Frame, area: Rect) {
        let selected_count = self.files.iter().filter(|item| item.selected).count();
        let selected_bytes: u64 = self
            .files
            .iter()
            .filter(|item| item.selected)
            .map(|item| item.file.size)
            .sum();

        let widths = [
            Constraint::Length(3),
            Constraint::Length(10),
            Constraint::Length(7),
            Constraint::Length(11),
            Constraint::Length(12),
            Constraint::Min(20),
        ];
        let header = Row::new(vec![
            Cell::from(""),
            Cell::from("quant"),
            Cell::from("quality"),
            Cell::from("size"),
            Cell::from("RAM (est)"),
            Cell::from("file"),
        ])
        .style(Style::new().fg(Color::Black).bg(Color::Magenta).bold());

        let rows: Vec<Row> = self
            .files
            .iter()
            .map(|item| {
                let mark = if item.selected { "[x]" } else { "[ ]" };
                let quality = item
                    .quant
                    .as_deref()
                    .map(stars)
                    .unwrap_or_else(|| "     ".to_string());
                let mut name = item.file.path.clone();
                if item
                    .quant
                    .as_deref()
                    .map(crate::api::filter::is_recommended)
                    .unwrap_or(false)
                {
                    name.push_str("  ★ recommended");
                }
                Row::new(vec![
                    Cell::from(mark),
                    Cell::from(item.quant.clone().unwrap_or_else(|| "—".to_string())),
                    Cell::from(quality),
                    Cell::from(util::format_bytes(item.file.size)),
                    Cell::from(format!(
                        "~{}",
                        util::format_bytes(ram_estimate(item.file.size))
                    )),
                    Cell::from(name),
                ])
                .style(if item.selected {
                    Style::new().fg(Color::Green)
                } else {
                    Style::new()
                })
            })
            .collect();

        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::bordered()
                    .title(format!(
                        " files · {selected_count} selected · {} ",
                        util::format_bytes(selected_bytes)
                    ))
                    .border_style(Style::new().fg(Color::Magenta)),
            )
            .row_highlight_style(Style::new().bg(Color::Indexed(236)).bold())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(table, area, &mut self.file_state);
    }

    fn draw_local(&mut self, frame: &mut Frame, area: Rect) {
        let header = Row::new(vec![
            Cell::from("repository"),
            Cell::from("files"),
            Cell::from("size"),
            Cell::from("downloaded"),
        ])
        .style(Style::new().fg(Color::Black).bg(Color::Green).bold());
        let rows: Vec<Row> = self
            .local
            .iter()
            .map(|(_, manifest)| {
                Row::new(vec![
                    Cell::from(manifest.repo.clone()),
                    Cell::from(manifest.files.len().to_string()),
                    Cell::from(util::format_bytes(manifest.total_size)),
                    Cell::from(manifest.downloaded_at.clone()),
                ])
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Min(30),
                Constraint::Length(6),
                Constraint::Length(12),
                Constraint::Length(22),
            ],
        )
        .header(header)
        .block(
            Block::bordered()
                .title(format!(
                    " local · {} ",
                    util::display_path(&self.settings.output_root)
                ))
                .border_style(Style::new().fg(Color::Green)),
        )
        .row_highlight_style(Style::new().bg(Color::Indexed(236)).bold())
        .highlight_symbol("▶ ");
        frame.render_stateful_widget(table, area, &mut self.local_state);
    }

    fn draw_local_detail(&self, frame: &mut Frame, area: Rect) {
        let Some((path, manifest)) = &self.local_detail else {
            return;
        };
        let mut lines = vec![
            Line::from(Span::styled(
                manifest.repo.clone(),
                Style::new().fg(Color::White).bold(),
            )),
            Line::from(format!(
                "path       {}",
                util::display_path(path.parent().unwrap_or(path))
            )),
            Line::from(format!("revision   {}", manifest.revision)),
            Line::from(format!("commit     {}", manifest.commit)),
            Line::from(format!("downloaded {}", manifest.downloaded_at)),
            Line::from(format!(
                "size       {}",
                util::format_bytes(manifest.total_size)
            )),
            Line::from("files"),
        ];
        for file in &manifest.files {
            lines.push(Line::from(format!(
                "  {:<56} {:>12}",
                util::truncate_end(&file.path, 56),
                util::format_bytes(file.size)
            )));
        }
        let block = Block::bordered()
            .title(" details ")
            .border_style(Style::new().fg(Color::Green));
        frame.render_widget(
            Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
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

/// Run the interactive browser until the user quits.
pub async fn run(
    client: Arc<HfClient>,
    settings: Settings,
    query: Option<String>,
    dataset: bool,
) -> Result<()> {
    let mut terminal = ratatui::init();
    let mut browser = Browser::new(client, settings);
    browser.dataset = dataset;
    if let Some(query) = query {
        browser.screen = Screen::Search;
        browser.search_focus = SearchFocus::Input;
        browser.input = query.clone();
        browser.status = "searching…".to_string();
        terminal.draw(|frame| browser.draw(frame))?;
        browser.do_search(&query).await;
    }
    let result = event_loop(&mut terminal, &mut browser).await;
    ratatui::restore();
    result
}

async fn event_loop(terminal: &mut DefaultTerminal, browser: &mut Browser) -> Result<()> {
    let mut reader = EventStream::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(150));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        browser.poll_session();
        terminal.draw(|frame| browser.draw(frame))?;

        tokio::select! {
            maybe_event = reader.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => {
                        match browser.on_key(key) {
                            Outcome::Quit => break,
                            Outcome::Search(query) => {
                                browser.status = "searching…".into();
                                terminal.draw(|frame| browser.draw(frame))?;
                                browser.do_search(&query).await;
                            }
                            Outcome::OpenRepo(repo) => {
                                browser.status = format!("loading {repo}…");
                                terminal.draw(|frame| browser.draw(frame))?;
                                browser.do_open_repo(&repo).await;
                            }
                            Outcome::None => {}
                        }
                    }
                    Some(Ok(Event::Resize(_, _))) => {}
                    Some(Err(_)) | None => {}
                    _ => {}
                }
            }
            _ = ticker.tick() => {}
        }

        if browser.quit {
            break;
        }
    }

    if let Some(session) = browser.session.take() {
        session.token.cancel();
        session.sampler.abort();
    }
    Ok(())
}
