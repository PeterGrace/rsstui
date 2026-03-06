//! Application-level error types.
//!
//! All fallible operations in `rsstui` propagate through this enum, keeping
//! error handling uniform and avoiding ad-hoc string errors in production code.

use thiserror::Error;

/// Errors that can occur anywhere in the `rsstui` application.
#[derive(Debug, Error)]
pub enum AppError {
    /// An HTTP request failed (network error, timeout, non-2xx status, …).
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// An RSS/Atom feed could not be parsed.
    #[error("Feed parse error: {0}")]
    Parse(String),

    /// A filesystem I/O operation failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization or deserialization failed.
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// Terminal setup or teardown failed.
    #[error("Terminal error: {0}")]
    Terminal(String),
}

// feed-rs's ParseFeedError implements Display + std::error::Error, so we
// convert it by capturing its message rather than using #[from] directly.
impl From<feed_rs::parser::ParseFeedError> for AppError {
    fn from(e: feed_rs::parser::ParseFeedError) -> Self {
        AppError::Parse(e.to_string())
    }
}
