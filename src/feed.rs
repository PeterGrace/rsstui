//! Feed data structures and asynchronous fetching logic.
//!
//! This module owns the `Article` and `FeedData` types that flow through the
//! application, plus the single async entry-point `fetch_feed` that converts a
//! URL into parsed content.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{error::AppError, markdown::html_to_markdown};

/// A single article or entry fetched from an RSS or Atom feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Article {
    /// Unique identifier for the article (`<id>` / `<guid>` from the feed).
    /// Used to persist read state across sessions.
    pub id: String,

    /// Article headline.
    pub title: String,

    /// Link to the full article on the web.
    pub link: Option<String>,

    /// Publication timestamp, when the feed supplies one.
    pub published: Option<DateTime<Utc>>,

    /// Short description, summary, or body excerpt.
    pub summary: String,

    /// Whether the user has marked this article as read.
    pub read: bool,
}

/// All data returned from a single successful feed fetch.
#[derive(Debug, Clone)]
pub struct FeedData {
    /// Resolved display title of the feed (falls back to URL if absent).
    pub title: String,

    /// Parsed articles, newest-first order as provided by the feed.
    pub articles: Vec<Article>,
}

/// Fetches and parses an RSS or Atom feed from the given URL.
///
/// Supports RSS 0.9x, RSS 1.0, RSS 2.0, Atom 0.3, Atom 1.0, and JSON Feed
/// (via the `feed-rs` crate).
///
/// # Arguments
///
/// * `url`    - The URL of the feed to fetch.
/// * `client` - A shared `reqwest::Client`; reuse it for connection pooling.
///
/// # Returns
///
/// A `FeedData` struct containing the resolved title and all parsed articles.
///
/// # Errors
///
/// * `AppError::Http`  — network failure, DNS error, or non-2xx HTTP status.
/// * `AppError::Parse` — the response body is not valid RSS/Atom/JSON Feed.
pub async fn fetch_feed(url: &str, client: &reqwest::Client) -> Result<FeedData, AppError> {
    let bytes = client
        .get(url)
        // Identify ourselves; some feed servers block requests without a UA.
        .header("User-Agent", "rsstui/0.1.0 (+https://github.com/rsstui)")
        .send()
        .await?
        // Propagate 4xx / 5xx as an error rather than attempting to parse HTML.
        .error_for_status()?
        .bytes()
        .await?;

    // `feed_rs::parser::parse` accepts any `impl Read`; `&[u8]` satisfies this.
    let feed = feed_rs::parser::parse(bytes.as_ref())?;

    let title = feed
        .title
        .map(|t| t.content)
        .unwrap_or_else(|| url.to_string());

    let articles = feed
        .entries
        .into_iter()
        .map(|entry| {
            let title = entry
                .title
                .map(|t| t.content)
                .unwrap_or_else(|| "(untitled)".to_string());

            // Take the first link as the canonical article URL.
            let link = entry.links.into_iter().next().map(|l| l.href);

            // Prefer summary; fall back to the content body if present.
            // Convert whatever HTML the feed provides to Markdown so the
            // preview pane can render it with proper styling.
            let summary = entry
                .summary
                .map(|s| s.content)
                .or_else(|| entry.content.and_then(|c| c.body))
                .map(|raw| html_to_markdown(&raw))
                .unwrap_or_default();

            Article {
                id: entry.id,
                title,
                link,
                published: entry.published,
                summary,
                read: false,
            }
        })
        .collect();

    Ok(FeedData { title, articles })
}
