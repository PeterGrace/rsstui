//! All ratatui rendering logic.
//!
//! The public surface is one function — `render` — that redraws the entire
//! terminal each frame.  All helper functions are private to this module.
//!
//! Layout (three panes + chrome):
//! ```text
//! ┌────────────────────────────────────────────────────────────────┐
//! │ rsstui                                           [q]uit        │  <- title (1 row)
//! ├─────────────────┬──────────────────────┬──────────────────────┤
//! │ Feeds           │ Articles             │ Preview              │
//! │                 │                      │                      │
//! │ > Hacker News   │ > Article headline   │ Full title           │
//! │   The Verge     │   Another article    │                      │
//! │   Ars Technica  │   ...                │ 2026-03-06 12:34 UTC │
//! │   ...           │                      │                      │
//! │                 │                      │ Summary / body...    │
//! └─────────────────┴──────────────────────┴──────────────────────┘
//! │ [Tab] pane  [a]add  [d]del  [r]refresh  [o]open  [m]mark      │  <- help (1 row)
//! └────────────────────────────────────────────────────────────────┘
//! ```

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use crate::app::{ActivePane, App, AppMode, StatusLevel};

// ── Colour palette ────────────────────────────────────────────────────────────

/// Border colour for the pane that currently has keyboard focus.
const ACTIVE_BORDER: Color = Color::Cyan;
/// Border colour for panes without focus.
const INACTIVE_BORDER: Color = Color::DarkGray;
/// Colour used for unread-count badges.
const UNREAD_COLOR: Color = Color::Green;
/// Colour used for error indicators.
const ERROR_COLOR: Color = Color::Red;
/// Colour used for "loading…" indicators.
const LOADING_COLOR: Color = Color::Yellow;

// ── Entry point ───────────────────────────────────────────────────────────────

/// Redraws the entire terminal for one frame.
///
/// Takes `&mut App` because `render_stateful_widget` mutates `ListState` to
/// track scroll offsets.
///
/// # Arguments
///
/// * `app`   - Mutable borrow of the full application state.
/// * `frame` - The ratatui `Frame` provided by `terminal.draw(…)`.
pub fn render(app: &mut App, frame: &mut Frame<'_>) {
    let area = frame.area();

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title bar
            Constraint::Min(0),    // three-pane body
            Constraint::Length(1), // help / status bar
        ])
        .split(area);

    render_title(app, frame, outer[0]);
    render_body(app, frame, outer[1]);
    render_status(app, frame, outer[2]);

    // Modal overlays are drawn last so they appear on top.
    match app.mode {
        AppMode::AddingFeed => render_add_feed_popup(app, frame, area),
        AppMode::ConfirmDelete => render_confirm_delete_popup(app, frame, area),
        AppMode::Normal => {}
    }
}

// ── Title bar ─────────────────────────────────────────────────────────────────

fn render_title(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let loading_indicator = if !app.loading.is_empty() {
        Span::styled(" [fetching...]", Style::default().fg(LOADING_COLOR))
    } else {
        Span::raw("")
    };

    let line = Line::from(vec![
        Span::styled(
            " rsstui ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  RSS Feed Reader"),
        loading_indicator,
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

// ── Three-pane body ───────────────────────────────────────────────────────────

fn render_body(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(35),
            Constraint::Percentage(45),
        ])
        .split(area);

    render_feeds_pane(app, frame, panes[0]);
    render_articles_pane(app, frame, panes[1]);
    render_preview_pane(app, frame, panes[2]);
}

// ── Feeds pane ────────────────────────────────────────────────────────────────

fn render_feeds_pane(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let is_active = app.active_pane == ActivePane::Feeds;
    let border_color = if is_active { ACTIVE_BORDER } else { INACTIVE_BORDER };

    let title = if is_active { " Feeds (active) " } else { " Feeds " };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title);

    let items: Vec<ListItem<'_>> = app
        .feeds
        .iter()
        .map(|feed| {
            let is_loading = app.loading.contains(&feed.url);
            let unread = feed.unread_count();

            // Build the status suffix shown after the feed title.
            let suffix = if is_loading {
                Span::styled(" [...]", Style::default().fg(LOADING_COLOR))
            } else if feed.fetch_error.is_some() {
                Span::styled(" [!]", Style::default().fg(ERROR_COLOR))
            } else if unread > 0 {
                Span::styled(
                    format!(" ({})", unread),
                    Style::default().fg(UNREAD_COLOR),
                )
            } else {
                Span::raw("")
            };

            let title_style = if unread > 0 {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            ListItem::new(Line::from(vec![
                Span::styled(feed.title.clone(), title_style),
                suffix,
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, &mut app.feed_list_state);
}

// ── Articles pane ─────────────────────────────────────────────────────────────

fn render_articles_pane(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let is_active = app.active_pane == ActivePane::Articles;
    let border_color = if is_active { ACTIVE_BORDER } else { INACTIVE_BORDER };

    let title = if is_active { " Articles (active) " } else { " Articles " };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title);

    let articles = app
        .feeds
        .get(app.selected_feed)
        .map(|f| f.articles.as_slice())
        .unwrap_or_default();

    if articles.is_empty() {
        let msg = if app
            .feeds
            .get(app.selected_feed)
            .map(|f| app.loading.contains(&f.url))
            .unwrap_or(false)
        {
            "Fetching articles..."
        } else if app.feeds.is_empty() {
            "No feeds — press [a] to add one"
        } else {
            "No articles. Press [r] to refresh."
        };

        frame.render_widget(
            Paragraph::new(msg)
                .block(block)
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    let items: Vec<ListItem<'_>> = articles
        .iter()
        .map(|article| {
            // Show date (if present) right-aligned by padding — using a simple
            // truncated title + date approach.
            let date_str = article
                .published
                .map(|dt| dt.format("%Y-%m-%d").to_string())
                .unwrap_or_default();

            let title_style = if article.read {
                Style::default().fg(Color::Gray)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };

            let read_marker = if article.read { "  " } else { "* " };

            ListItem::new(Line::from(vec![
                Span::styled(read_marker, Style::default().fg(UNREAD_COLOR)),
                Span::styled(article.title.clone(), title_style),
                Span::styled(
                    format!("  {}", date_str),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, &mut app.article_list_state);
}

// ── Preview pane ──────────────────────────────────────────────────────────────

fn render_preview_pane(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let is_active = app.active_pane == ActivePane::Preview;
    let border_color = if is_active { ACTIVE_BORDER } else { INACTIVE_BORDER };

    let title = if is_active { " Preview (active) " } else { " Preview " };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title);

    let Some(article) = app
        .feeds
        .get(app.selected_feed)
        .and_then(|f| f.articles.get(app.selected_article))
    else {
        frame.render_widget(
            Paragraph::new("Select an article to preview.")
                .block(block)
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    };

    // Build a rich `Text` with multiple styled lines.
    let mut lines: Vec<Line<'_>> = Vec::new();

    // Title (bold, wrapped)
    lines.push(Line::from(Span::styled(
        article.title.clone(),
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::White),
    )));
    lines.push(Line::from(""));

    // Published date
    if let Some(dt) = article.published {
        lines.push(Line::from(vec![
            Span::styled("Published: ", Style::default().fg(Color::DarkGray)),
            Span::raw(dt.format("%Y-%m-%d %H:%M UTC").to_string()),
        ]));
    }

    // Link
    if let Some(link) = &article.link {
        lines.push(Line::from(vec![
            Span::styled("Link: ", Style::default().fg(Color::DarkGray)),
            Span::styled(link.clone(), Style::default().fg(Color::Cyan)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    // Summary / body text — split on newlines so wrapping works per paragraph.
    for para in article.summary.lines() {
        lines.push(Line::from(para.to_string()));
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((app.preview_scroll, 0)),
        area,
    );
}

// ── Status / help bar ─────────────────────────────────────────────────────────

fn render_status(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let widget = if let Some((msg, level)) = &app.status {
        let color = match level {
            StatusLevel::Info => Color::Green,
            StatusLevel::Error => Color::Red,
        };
        Paragraph::new(format!(" {msg}")).style(Style::default().fg(color))
    } else {
        // Context-sensitive help text.
        let help = match app.mode {
            AppMode::AddingFeed => " [Enter] confirm  [Esc] cancel",
            AppMode::ConfirmDelete => " Delete feed? [y] yes  [any] no",
            AppMode::Normal => match app.active_pane {
                ActivePane::Feeds => {
                    " [Tab] focus  [a] add  [d] delete  [r] refresh  [R] refresh all  [q] quit"
                }
                ActivePane::Articles => {
                    " [Tab] focus  [Enter] preview  [m] toggle read  [o] open  [r] refresh  [q] quit"
                }
                ActivePane::Preview => {
                    " [Tab] focus  [j/k] scroll  [u/d] scroll 5  [m] toggle read  [o] open  [q] quit"
                }
            },
        };
        Paragraph::new(help).style(Style::default().fg(Color::DarkGray))
    };

    frame.render_widget(widget, area);
}

// ── Modal: add feed ───────────────────────────────────────────────────────────

fn render_add_feed_popup(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let popup_area = centered_rect(60, 20, area);

    // Clears whatever was rendered behind the popup.
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Add Feed — enter URL then press Enter ");

    // Render the border/title first.
    frame.render_widget(block, popup_area);

    // The inner area for the text input (inset by the border).
    let inner = Rect {
        x: popup_area.x + 1,
        y: popup_area.y + 1,
        width: popup_area.width.saturating_sub(2),
        height: popup_area.height.saturating_sub(2),
    };

    // Build a line that highlights the character under the cursor.
    let cursor_line = build_cursor_line(&app.input_buffer, app.input_cursor);
    frame.render_widget(Paragraph::new(cursor_line), inner);
}

/// Builds a `Line` that shows `buffer` with the character at `cursor` visually
/// inverted so the user can see where text will be inserted.
fn build_cursor_line<'a>(buffer: &'a str, cursor: usize) -> Line<'a> {
    let cursor = cursor.min(buffer.len());

    // Split at the cursor byte position.  Since we only ever insert ASCII
    // characters (URLs), this is safe to do at byte boundaries.
    let before = &buffer[..cursor];

    if cursor < buffer.len() {
        // Highlight the character under the cursor.
        let char_len = buffer[cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| i)
            .unwrap_or(buffer.len() - cursor);
        let under = &buffer[cursor..cursor + char_len];
        let after = &buffer[cursor + char_len..];

        Line::from(vec![
            Span::raw(before),
            Span::styled(under, Style::default().bg(Color::White).fg(Color::Black)),
            Span::raw(after),
        ])
    } else {
        // Cursor is past the end — append a highlighted space as a caret.
        Line::from(vec![
            Span::raw(before),
            Span::styled(" ", Style::default().bg(Color::White).fg(Color::Black)),
        ])
    }
}

// ── Modal: confirm delete ─────────────────────────────────────────────────────

fn render_confirm_delete_popup(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let popup_area = centered_rect(50, 25, area);

    frame.render_widget(Clear, popup_area);

    let feed_url = app
        .feeds
        .get(app.selected_feed)
        .map(|f| f.title.as_str())
        .unwrap_or("(unknown)");

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ERROR_COLOR))
        .title(" Confirm Delete ");

    let text = Text::from(vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("Delete feed:  "),
            Span::styled(feed_url, Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  [y] Yes   [any other key] Cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ]);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(Paragraph::new(text).block(block), popup_area);
}

// ── Layout helper ─────────────────────────────────────────────────────────────

/// Returns a `Rect` centered in `area` with the given percentage dimensions.
///
/// # Arguments
///
/// * `percent_x` - Desired width as a percentage of `area.width`.
/// * `percent_y` - Desired height as a percentage of `area.height`.
/// * `area`      - The bounding rectangle to center within.
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
