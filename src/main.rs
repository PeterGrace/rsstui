//! Entry point for `rsstui` — a terminal-based RSS/Atom feed reader.
//!
//! Responsibilities:
//! * Bootstrap the `tracing` subscriber for structured logging.
//! * Install a panic hook that restores the terminal before printing the
//!   panic message, so the user's shell is not left in raw mode.
//! * Initialise the ratatui/crossterm terminal.
//! * Create the `App` and trigger an initial refresh of all subscribed feeds.
//! * Run the async event loop (`tokio::select!` over keyboard events and a
//!   periodic tick that drives background message processing).
//! * Restore the terminal on normal and abnormal exit.

mod app;
mod error;
mod feed;
mod markdown;
mod storage;
mod ui;

use std::io;
use std::time::Duration;

use crossterm::{
    event::{Event, EventStream, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use tracing_subscriber::{EnvFilter, Registry, prelude::*};

use app::App;
use error::AppError;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env if present (allows e.g. RUST_LOG overrides without touching the shell).
    let _ = dotenvy::dotenv();

    // Structured logging — level controlled by the RUST_LOG environment variable.
    // Defaults to "warn" so only important messages are shown (tracing output
    // goes to a file or is suppressed in TUI mode anyway).
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn"));

    Registry::default()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Install a panic hook that tears down the terminal before printing the
    // panic message.  Without this, a panic leaves the terminal in raw mode.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort: ignore errors during emergency cleanup.
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen);
        original_hook(info);
    }));

    // ── Terminal setup ────────────────────────────────────────────────────────
    enable_raw_mode().map_err(|e| AppError::Terminal(e.to_string()))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|e| AppError::Terminal(e.to_string()))?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal =
        Terminal::new(backend).map_err(|e| AppError::Terminal(e.to_string()))?;
    terminal.hide_cursor().map_err(|e| AppError::Terminal(e.to_string()))?;

    // ── App initialisation ────────────────────────────────────────────────────
    let mut app = App::new()?;

    // Kick off a background refresh for every already-subscribed feed so the
    // user sees fresh content immediately on launch.
    if !app.feeds.is_empty() {
        app.refresh_all();
    }

    // ── Event loop ────────────────────────────────────────────────────────────
    // `EventStream` is an async `Stream` over crossterm `Event`s.
    // `tokio::time::interval` provides periodic ticks for background work.
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));

    let result = run_loop(&mut app, &mut terminal, &mut events, &mut tick).await;

    // ── Terminal teardown ─────────────────────────────────────────────────────
    // Always attempt to restore the terminal, even if the event loop errored.
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    // Surface any error from the event loop after the terminal is restored.
    result?;
    Ok(())
}

/// Runs the main event loop until `app.should_quit` is set.
///
/// Uses `tokio::select!` to interleave keyboard events and periodic ticks
/// without blocking the tokio runtime (both sources are async).
async fn run_loop(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    events: &mut EventStream,
    tick: &mut tokio::time::Interval,
) -> Result<(), AppError> {
    loop {
        // Draw the current frame before blocking on the next event.
        terminal
            .draw(|frame| ui::render(app, frame))
            .map_err(|e| AppError::Terminal(e.to_string()))?;

        tokio::select! {
            biased; // prefer keyboard events over timer ticks

            // A crossterm event arrived.
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) => {
                        // Ignore key-release events on platforms that emit them
                        // (e.g. Windows).  We only act on Press and Repeat.
                        if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat {
                            app.handle_key(key);
                        }
                    }
                    // Terminal resize — ratatui handles it automatically on the
                    // next draw call, so we just need to trigger a redraw.
                    Some(Ok(Event::Resize(_, _))) => {}
                    // Stream ended (terminal closed) — exit cleanly.
                    None => break,
                    // Crossterm I/O error — propagate.
                    Some(Err(e)) => return Err(AppError::Io(e)),
                    _ => {}
                }
            }

            // Periodic tick — process results from background fetch tasks.
            _ = tick.tick() => {
                app.poll_messages();
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
