//! Application state and keyboard event handling.
//!
//! `App` is the single source of truth for every piece of UI state: which pane
//! has focus, which feed / article is selected, the text typed in the "add
//! feed" modal, and the list of in-flight HTTP fetches.  Background fetch tasks
//! communicate with the main loop via an unbounded tokio channel.

use std::{collections::HashSet, time::Duration};

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use tokio::sync::mpsc;

use crate::{
    error::AppError,
    feed::{Article, FeedData, fetch_feed},
    storage::{FeedConfig, StorageConfig, load_config, save_config},
};

// ── UI state enums ────────────────────────────────────────────────────────────

/// Which of the three panes currently receives keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePane {
    Feeds,
    Articles,
    Preview,
}

/// High-level application mode (normal navigation vs. a modal overlay).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    /// Normal three-pane navigation.
    Normal,
    /// The "add feed" text-entry dialog is open.
    AddingFeed,
    /// The "confirm delete feed" dialog is open.
    ConfirmDelete,
}

/// Severity level for transient status-bar messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusLevel {
    Info,
    Error,
}

// ── Domain types ──────────────────────────────────────────────────────────────

/// A feed subscription held in memory, combining stored config and live data.
#[derive(Debug, Clone)]
pub struct FeedEntry {
    /// The subscribed feed URL.
    pub url: String,
    /// Display title resolved after the first successful fetch; falls back to URL.
    pub title: String,
    /// Parsed articles — empty until the first successful fetch.
    pub articles: Vec<Article>,
    /// Timestamp of the most recent successful fetch.
    pub last_refreshed: Option<DateTime<Utc>>,
    /// Error message from the most recent failed fetch, if any.
    pub fetch_error: Option<String>,
    /// Article IDs the user has marked as read (persisted across sessions).
    pub read_ids: HashSet<String>,
}

impl FeedEntry {
    /// Creates a `FeedEntry` from a stored config, with no live data yet.
    pub fn from_config(config: &FeedConfig) -> Self {
        Self {
            url: config.url.clone(),
            title: config.url.clone(),
            articles: Vec::new(),
            last_refreshed: None,
            fetch_error: None,
            read_ids: config.read_ids.clone(),
        }
    }

    /// Returns the number of unread articles in this feed.
    pub fn unread_count(&self) -> usize {
        self.articles.iter().filter(|a| !a.read).count()
    }

    /// Converts the entry's mutable state back into a `FeedConfig` for disk
    /// persistence (retains URL and read IDs; drops transient live data).
    pub fn to_config(&self) -> FeedConfig {
        FeedConfig {
            url: self.url.clone(),
            read_ids: self.read_ids.clone(),
        }
    }
}

// ── Async messaging ───────────────────────────────────────────────────────────

/// Messages sent from background fetch tasks to the main event loop.
pub enum FeedMessage {
    /// A feed fetch completed — successfully or with an error.
    FetchDone {
        url: String,
        result: Result<FeedData, String>,
    },
}

// ── App struct ────────────────────────────────────────────────────────────────

/// All application state.
///
/// Held on the stack in `main`; never shared across threads.
/// Background tasks communicate solely through `msg_tx` / `msg_rx`.
pub struct App {
    // ── UI state ─────────────────────────────────────────────────────────────
    pub active_pane: ActivePane,
    pub mode: AppMode,
    pub should_quit: bool,

    // ── Feed list ─────────────────────────────────────────────────────────────
    pub feeds: Vec<FeedEntry>,
    /// Index into `feeds` of the highlighted feed.
    pub selected_feed: usize,
    /// ratatui list state for the feeds panel (tracks scroll offset).
    pub feed_list_state: ListState,

    // ── Article list ──────────────────────────────────────────────────────────
    /// Index into `feeds[selected_feed].articles` of the highlighted article.
    pub selected_article: usize,
    /// ratatui list state for the articles panel (tracks scroll offset).
    pub article_list_state: ListState,

    // ── Preview pane ──────────────────────────────────────────────────────────
    /// Vertical scroll offset (in rows) for the preview paragraph.
    pub preview_scroll: u16,

    // ── Add-feed modal ────────────────────────────────────────────────────────
    /// Text typed so far in the add-feed dialog.
    pub input_buffer: String,
    /// Byte-offset of the insertion cursor within `input_buffer`.
    pub input_cursor: usize,

    // ── Status bar ────────────────────────────────────────────────────────────
    /// Transient message shown in the status bar; cleared on next action.
    pub status: Option<(String, StatusLevel)>,

    // ── Async infra ───────────────────────────────────────────────────────────
    /// Feed URLs currently being fetched in the background.
    pub loading: HashSet<String>,
    /// Cloned and given to each spawned task so results flow back here.
    pub msg_tx: mpsc::UnboundedSender<FeedMessage>,
    /// Polled every event-loop iteration via `try_recv`.
    pub msg_rx: mpsc::UnboundedReceiver<FeedMessage>,
    /// Shared HTTP client — cheap to clone; backed by a connection pool.
    pub http: reqwest::Client,
}

impl App {
    /// Creates a new `App`, loading any existing feed subscriptions from disk.
    ///
    /// # Errors
    ///
    /// * `AppError::Io` / `AppError::Serde` — storage load fails.
    /// * `AppError::Http` — the `reqwest::Client` cannot be constructed.
    pub fn new() -> Result<Self, AppError> {
        let (msg_tx, msg_rx) = mpsc::unbounded_channel();

        let config = load_config()?;

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(AppError::Http)?;

        let feeds: Vec<FeedEntry> = config.feeds.iter().map(FeedEntry::from_config).collect();

        // Pre-select the first feed and first article if they exist.
        let mut feed_list_state = ListState::default();
        let mut article_list_state = ListState::default();
        if !feeds.is_empty() {
            feed_list_state.select(Some(0));
            if !feeds[0].articles.is_empty() {
                article_list_state.select(Some(0));
            }
        }

        Ok(Self {
            active_pane: ActivePane::Feeds,
            mode: AppMode::Normal,
            should_quit: false,
            feeds,
            selected_feed: 0,
            feed_list_state,
            selected_article: 0,
            article_list_state,
            preview_scroll: 0,
            input_buffer: String::new(),
            input_cursor: 0,
            status: None,
            loading: HashSet::new(),
            msg_tx,
            msg_rx,
            http,
        })
    }

    // ── Persistence ───────────────────────────────────────────────────────────

    /// Writes the current feed list (URLs + read IDs) to disk.
    ///
    /// Errors are logged but not propagated — a save failure must not crash the
    /// interactive UI.
    fn save(&self) {
        let config = StorageConfig {
            feeds: self.feeds.iter().map(FeedEntry::to_config).collect(),
        };
        if let Err(e) = save_config(&config) {
            tracing::error!("Failed to save config: {e}");
        }
    }

    // ── Async fetch ───────────────────────────────────────────────────────────

    /// Spawns a background task to (re-)fetch the feed at `url`.
    ///
    /// Does nothing if a fetch for `url` is already in flight.
    pub fn spawn_fetch(&mut self, url: String) {
        if self.loading.contains(&url) {
            return;
        }
        self.loading.insert(url.clone());

        let tx = self.msg_tx.clone();
        let client = self.http.clone();

        tokio::spawn(async move {
            let result = fetch_feed(&url, &client)
                .await
                .map_err(|e| e.to_string());
            // If the receiver is gone the app is already exiting — ignore the error.
            let _ = tx.send(FeedMessage::FetchDone { url, result });
        });
    }

    /// Triggers a background refresh of all subscribed feeds.
    pub fn refresh_all(&mut self) {
        let urls: Vec<String> = self.feeds.iter().map(|f| f.url.clone()).collect();
        for url in urls {
            self.spawn_fetch(url);
        }
        self.set_status("Refreshing all feeds...", StatusLevel::Info);
    }

    /// Triggers a background refresh of the currently selected feed, if any.
    pub fn refresh_selected(&mut self) {
        if let Some(feed) = self.feeds.get(self.selected_feed) {
            let url = feed.url.clone();
            self.spawn_fetch(url);
            self.set_status("Refreshing feed...", StatusLevel::Info);
        }
    }

    /// Drains any pending messages from background fetch tasks and applies them
    /// to the feed list.  Called every event-loop tick.
    pub fn poll_messages(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                FeedMessage::FetchDone { url, result } => {
                    self.loading.remove(&url);

                    // Find the matching feed by URL.
                    let Some((idx, feed)) = self
                        .feeds
                        .iter_mut()
                        .enumerate()
                        .find(|(_, f)| f.url == url)
                    else {
                        continue;
                    };

                    match result {
                        Ok(data) => {
                            feed.title = data.title;
                            feed.last_refreshed = Some(Utc::now());
                            feed.fetch_error = None;

                            // Merge incoming articles, restoring persisted read state.
                            feed.articles = data
                                .articles
                                .into_iter()
                                .map(|mut a| {
                                    a.read = feed.read_ids.contains(&a.id);
                                    a
                                })
                                .collect();

                            // If this is the selected feed, ensure the article
                            // list state points at a valid index.
                            if idx == self.selected_feed {
                                let has = !feed.articles.is_empty();
                                let sel = if has { Some(0) } else { None };
                                if self.article_list_state.selected().is_none() && has {
                                    self.selected_article = 0;
                                    self.article_list_state.select(sel);
                                }
                            }
                        }
                        Err(e) => {
                            feed.fetch_error = Some(e.clone());
                            self.status = Some((
                                format!("Fetch error for {url}: {e}"),
                                StatusLevel::Error,
                            ));
                        }
                    }
                }
            }
        }

        self.save();
    }

    // ── Status bar ────────────────────────────────────────────────────────────

    /// Sets the status bar message shown at the bottom of the screen.
    pub fn set_status(&mut self, msg: &str, level: StatusLevel) {
        self.status = Some((msg.to_string(), level));
    }

    // ── Read state ────────────────────────────────────────────────────────────

    /// Marks the currently selected article as read or unread.
    ///
    /// Also updates the persisted `read_ids` set and saves to disk.
    pub fn mark_read(&mut self, read: bool) {
        let Some(feed) = self.feeds.get_mut(self.selected_feed) else {
            return;
        };
        let Some(article) = feed.articles.get_mut(self.selected_article) else {
            return;
        };
        article.read = read;
        if read {
            feed.read_ids.insert(article.id.clone());
        } else {
            feed.read_ids.remove(&article.id);
        }
        self.save();
    }

    // ── Event dispatch ────────────────────────────────────────────────────────

    /// Routes a `KeyEvent` to the appropriate handler for the current mode.
    pub fn handle_key(&mut self, key: KeyEvent) {
        match &self.mode {
            AppMode::Normal => self.handle_key_normal(key),
            AppMode::AddingFeed => self.handle_key_adding(key),
            AppMode::ConfirmDelete => self.handle_key_confirm_delete(key),
        }
    }

    // ── Normal-mode key handling ──────────────────────────────────────────────

    fn handle_key_normal(&mut self, key: KeyEvent) {
        // Clear the status bar on any keypress.
        self.status = None;

        match key.code {
            // Quit
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }

            // Pane focus cycling
            KeyCode::Tab => {
                self.active_pane = match self.active_pane {
                    ActivePane::Feeds => ActivePane::Articles,
                    ActivePane::Articles => ActivePane::Preview,
                    ActivePane::Preview => ActivePane::Feeds,
                };
            }
            KeyCode::BackTab => {
                self.active_pane = match self.active_pane {
                    ActivePane::Feeds => ActivePane::Preview,
                    ActivePane::Articles => ActivePane::Feeds,
                    ActivePane::Preview => ActivePane::Articles,
                };
            }

            // Navigation
            KeyCode::Char('j') | KeyCode::Down => self.navigate_down(),
            KeyCode::Char('k') | KeyCode::Up => self.navigate_up(),
            KeyCode::Char('g') => self.navigate_top(),
            KeyCode::Char('G') => self.navigate_bottom(),

            // Feed-pane actions
            KeyCode::Char('a') => {
                self.mode = AppMode::AddingFeed;
                self.input_buffer.clear();
                self.input_cursor = 0;
            }
            KeyCode::Char('d') if self.active_pane == ActivePane::Feeds => {
                if !self.feeds.is_empty() {
                    self.mode = AppMode::ConfirmDelete;
                }
            }
            KeyCode::Char('r') => self.refresh_selected(),
            KeyCode::Char('R') => self.refresh_all(),

            // Article actions
            KeyCode::Enter => self.handle_enter(),
            KeyCode::Char('m') => {
                let is_read = self
                    .feeds
                    .get(self.selected_feed)
                    .and_then(|f| f.articles.get(self.selected_article))
                    .map(|a| a.read)
                    .unwrap_or(false);
                self.mark_read(!is_read);
            }
            KeyCode::Char('o') => self.open_in_browser(),

            // Preview scroll shortcuts (work in any pane)
            KeyCode::Char('u') => {
                self.preview_scroll = self.preview_scroll.saturating_sub(5);
            }
            KeyCode::Char('d') => {
                self.preview_scroll = self.preview_scroll.saturating_add(5);
            }

            _ => {}
        }
    }

    // ── Add-feed modal key handling ───────────────────────────────────────────

    fn handle_key_adding(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
                self.input_buffer.clear();
            }
            KeyCode::Enter => {
                let url = self.input_buffer.trim().to_string();
                if !url.is_empty() && !self.feeds.iter().any(|f| f.url == url) {
                    let entry = FeedEntry {
                        url: url.clone(),
                        title: url.clone(),
                        articles: Vec::new(),
                        last_refreshed: None,
                        fetch_error: None,
                        read_ids: HashSet::new(),
                    };
                    self.feeds.push(entry);
                    self.save();

                    // Select the newly added feed and kick off a fetch.
                    self.selected_feed = self.feeds.len() - 1;
                    self.feed_list_state.select(Some(self.selected_feed));
                    self.article_list_state.select(None);
                    self.selected_article = 0;

                    self.spawn_fetch(url);
                }
                self.mode = AppMode::Normal;
                self.input_buffer.clear();
                self.input_cursor = 0;
            }
            KeyCode::Backspace => {
                if self.input_cursor > 0 {
                    self.input_cursor -= 1;
                    self.input_buffer.remove(self.input_cursor);
                }
            }
            KeyCode::Delete => {
                if self.input_cursor < self.input_buffer.len() {
                    self.input_buffer.remove(self.input_cursor);
                }
            }
            KeyCode::Left => {
                self.input_cursor = self.input_cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                if self.input_cursor < self.input_buffer.len() {
                    self.input_cursor += 1;
                }
            }
            KeyCode::Home => self.input_cursor = 0,
            KeyCode::End => self.input_cursor = self.input_buffer.len(),
            KeyCode::Char(c) => {
                self.input_buffer.insert(self.input_cursor, c);
                self.input_cursor += 1;
            }
            _ => {}
        }
    }

    // ── Confirm-delete modal key handling ─────────────────────────────────────

    fn handle_key_confirm_delete(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if !self.feeds.is_empty() {
                    self.feeds.remove(self.selected_feed);
                    if self.selected_feed > 0 && self.selected_feed >= self.feeds.len() {
                        self.selected_feed -= 1;
                    }
                    let has = !self.feeds.is_empty();
                    self.feed_list_state
                        .select(if has { Some(self.selected_feed) } else { None });
                    self.selected_article = 0;
                    self.article_list_state.select(None);
                    self.preview_scroll = 0;
                    self.save();
                }
                self.mode = AppMode::Normal;
            }
            _ => {
                self.mode = AppMode::Normal;
            }
        }
    }

    // ── Navigation helpers ────────────────────────────────────────────────────

    fn navigate_down(&mut self) {
        match self.active_pane {
            ActivePane::Feeds => {
                if !self.feeds.is_empty() && self.selected_feed + 1 < self.feeds.len() {
                    self.selected_feed += 1;
                    self.feed_list_state.select(Some(self.selected_feed));
                    self.selected_article = 0;
                    let has_articles = !self.feeds[self.selected_feed].articles.is_empty();
                    self.article_list_state
                        .select(if has_articles { Some(0) } else { None });
                    self.preview_scroll = 0;
                }
            }
            ActivePane::Articles => {
                let len = self
                    .feeds
                    .get(self.selected_feed)
                    .map(|f| f.articles.len())
                    .unwrap_or(0);
                if len > 0 && self.selected_article + 1 < len {
                    self.selected_article += 1;
                    self.article_list_state.select(Some(self.selected_article));
                    self.preview_scroll = 0;
                }
            }
            ActivePane::Preview => {
                self.preview_scroll = self.preview_scroll.saturating_add(3);
            }
        }
    }

    fn navigate_up(&mut self) {
        match self.active_pane {
            ActivePane::Feeds => {
                if self.selected_feed > 0 {
                    self.selected_feed -= 1;
                    self.feed_list_state.select(Some(self.selected_feed));
                    self.selected_article = 0;
                    let has_articles = !self.feeds[self.selected_feed].articles.is_empty();
                    self.article_list_state
                        .select(if has_articles { Some(0) } else { None });
                    self.preview_scroll = 0;
                }
            }
            ActivePane::Articles => {
                if self.selected_article > 0 {
                    self.selected_article -= 1;
                    self.article_list_state.select(Some(self.selected_article));
                    self.preview_scroll = 0;
                }
            }
            ActivePane::Preview => {
                self.preview_scroll = self.preview_scroll.saturating_sub(3);
            }
        }
    }

    fn navigate_top(&mut self) {
        match self.active_pane {
            ActivePane::Feeds => {
                self.selected_feed = 0;
                self.feed_list_state.select(if self.feeds.is_empty() {
                    None
                } else {
                    Some(0)
                });
            }
            ActivePane::Articles => {
                self.selected_article = 0;
                let has = self
                    .feeds
                    .get(self.selected_feed)
                    .map(|f| !f.articles.is_empty())
                    .unwrap_or(false);
                self.article_list_state.select(if has { Some(0) } else { None });
            }
            ActivePane::Preview => {
                self.preview_scroll = 0;
            }
        }
    }

    fn navigate_bottom(&mut self) {
        match self.active_pane {
            ActivePane::Feeds => {
                if !self.feeds.is_empty() {
                    self.selected_feed = self.feeds.len() - 1;
                    self.feed_list_state.select(Some(self.selected_feed));
                }
            }
            ActivePane::Articles => {
                let len = self
                    .feeds
                    .get(self.selected_feed)
                    .map(|f| f.articles.len())
                    .unwrap_or(0);
                if len > 0 {
                    self.selected_article = len - 1;
                    self.article_list_state.select(Some(self.selected_article));
                }
            }
            ActivePane::Preview => {
                // No-op — preview has no known bottom boundary until rendered.
            }
        }
    }

    /// Handles the Enter key: advance focus or open article in preview.
    fn handle_enter(&mut self) {
        match self.active_pane {
            ActivePane::Feeds => {
                // Jump focus to the articles pane.
                self.active_pane = ActivePane::Articles;
                self.selected_article = 0;
                let has = self
                    .feeds
                    .get(self.selected_feed)
                    .map(|f| !f.articles.is_empty())
                    .unwrap_or(false);
                self.article_list_state.select(if has { Some(0) } else { None });
                self.preview_scroll = 0;
            }
            ActivePane::Articles => {
                let has = self
                    .feeds
                    .get(self.selected_feed)
                    .map(|f| !f.articles.is_empty())
                    .unwrap_or(false);
                if has {
                    self.active_pane = ActivePane::Preview;
                    self.preview_scroll = 0;
                    // Mark the article as read when the user opens it.
                    self.mark_read(true);
                }
            }
            ActivePane::Preview => {}
        }
    }

    /// Opens the selected article's URL in the system default browser.
    fn open_in_browser(&self) {
        let Some(url) = self
            .feeds
            .get(self.selected_feed)
            .and_then(|f| f.articles.get(self.selected_article))
            .and_then(|a| a.link.as_deref())
        else {
            return;
        };

        // xdg-open on Linux; open on macOS.
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
}
