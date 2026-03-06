//! HTML-to-Markdown conversion and Markdown-to-ratatui rendering.
//!
//! # Pipeline
//!
//! ```text
//! raw HTML  ──(htmd)──►  Markdown string  ──(pulldown-cmark)──►  ratatui Text
//! ```
//!
//! `html_to_markdown` runs at fetch time (once per article), storing clean
//! Markdown in `Article::summary`.  `render_markdown` runs at render time,
//! converting that Markdown into styled `Span`s that ratatui can display.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};

// ── Public API ────────────────────────────────────────────────────────────────

/// Converts an HTML fragment or document to Markdown.
///
/// Handles the elements most common in RSS feeds: `<br>`, `<p>`, `<h1>`–`<h6>`,
/// `<strong>` / `<b>`, `<em>` / `<i>`, `<a>`, `<ul>`, `<ol>`, `<li>`,
/// `<code>`, `<pre>`, `<blockquote>`, and `<hr>`.
///
/// # Arguments
///
/// * `html` — Raw HTML string (fragment or full document).
///
/// # Returns
///
/// Markdown string.  Returns `html` unchanged if `htmd` fails — the caller
/// should treat this as best-effort.
pub fn html_to_markdown(html: &str) -> String {
    htmd::convert(html).unwrap_or_else(|_| html.to_string())
}

/// Renders a Markdown string into a styled ratatui [`Text`].
///
/// Supported elements:
///
/// * **Headings** H1–H3 (distinct colours), H4–H6 (bold)
/// * **Strong** (`**text**`) — bold
/// * **Emphasis** (`*text*`) — italic
/// * **Strikethrough** (`~~text~~`) — crossed-out
/// * **Inline code** (`` `code` ``) — green
/// * **Fenced / indented code blocks** — green, bordered with horizontal rules
/// * **Unordered lists** — bullet `•`
/// * **Ordered lists** — numbered
/// * **Blockquotes** — grey italic with `|` gutter
/// * **Links** — cyan underlined (URL is not appended; feeds are noisy enough)
/// * **Images** — replaced with `[image]` placeholder
/// * **Horizontal rules**
///
/// # Arguments
///
/// * `md` — Markdown source text.
///
/// # Returns
///
/// `Text<'static>` ready to hand directly to a [`Paragraph`] widget.
pub fn render_markdown(md: &str) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    // Spans accumulating for the line currently being built.
    let mut current: Vec<Span<'static>> = Vec::new();

    // Style stack.  Each entry is the *fully resolved* style at that nesting
    // depth; inline events inherit from the top of the stack.
    let mut style_stack: Vec<Style> = vec![Style::default()];

    let mut in_code_block = false;
    // When true, suppress Text events (e.g. alt-text inside an image tag).
    let mut in_image = false;
    // List nesting: each frame tracks whether the list is ordered and, if so,
    // the next item number to emit.
    let mut list_stack: Vec<ListKind> = Vec::new();

    let parser = Parser::new_ext(md, Options::all());

    for event in parser {
        match event {
            // ── Block openers ─────────────────────────────────────────────────

            Event::Start(Tag::Heading(level, _, _)) => {
                flush(&mut lines, &mut current);
                // Headings override the ambient style entirely.
                style_stack.push(heading_style(level));
            }

            Event::Start(Tag::Paragraph) => {
                // Nothing to do on open; content follows as inline events.
            }

            Event::Start(Tag::BlockQuote) => {
                let inherited = top(&style_stack);
                style_stack.push(inherited.fg(Color::DarkGray).add_modifier(Modifier::ITALIC));
                // Visual gutter marker at the start of the first line.
                current.push(Span::styled("| ", Style::default().fg(Color::DarkGray)));
            }

            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
                flush(&mut lines, &mut current);
                lines.push(hr_line());
                style_stack.push(Style::default().fg(Color::Green));
            }

            Event::Start(Tag::List(ordered)) => {
                list_stack.push(match ordered {
                    Some(start) => ListKind::Ordered(start),
                    None => ListKind::Unordered,
                });
            }

            Event::Start(Tag::Item) => {
                // Indent each extra level of nesting by two spaces.
                let depth = list_stack.len().saturating_sub(1);
                let indent = "  ".repeat(depth);
                let bullet = match list_stack.last_mut() {
                    Some(ListKind::Ordered(ref mut n)) => {
                        let s = format!("{indent}{}. ", n);
                        *n += 1;
                        s
                    }
                    _ => format!("{indent}• "),
                };
                current.push(Span::styled(bullet, Style::default().fg(Color::Yellow)));
            }

            // ── Inline openers ────────────────────────────────────────────────

            Event::Start(Tag::Emphasis) => {
                style_stack.push(top(&style_stack).add_modifier(Modifier::ITALIC));
            }

            Event::Start(Tag::Strong) => {
                style_stack.push(top(&style_stack).add_modifier(Modifier::BOLD));
            }

            Event::Start(Tag::Strikethrough) => {
                style_stack.push(top(&style_stack).add_modifier(Modifier::CROSSED_OUT));
            }

            Event::Start(Tag::Link(_, _, _)) => {
                // Inherit existing modifiers (e.g. bold inside a link) but
                // switch the foreground colour to cyan + underline.
                style_stack.push(top(&style_stack).fg(Color::Cyan).add_modifier(Modifier::UNDERLINED));
            }

            Event::Start(Tag::Image(_, _, _)) => {
                in_image = true;
                current.push(Span::styled("[image]", Style::default().fg(Color::DarkGray)));
            }

            // ── Block closers ─────────────────────────────────────────────────

            Event::End(Tag::Heading(_, _, _)) => {
                style_stack.pop();
                flush(&mut lines, &mut current);
                lines.push(Line::from(""));
            }

            Event::End(Tag::Paragraph) => {
                flush(&mut lines, &mut current);
                lines.push(Line::from(""));
            }

            Event::End(Tag::BlockQuote) => {
                style_stack.pop();
                flush(&mut lines, &mut current);
                lines.push(Line::from(""));
            }

            Event::End(Tag::CodeBlock(_)) => {
                in_code_block = false;
                style_stack.pop();
                flush(&mut lines, &mut current);
                lines.push(hr_line());
                lines.push(Line::from(""));
            }

            Event::End(Tag::List(_)) => {
                list_stack.pop();
                // Add a blank line after top-level lists only.
                if list_stack.is_empty() {
                    lines.push(Line::from(""));
                }
            }

            Event::End(Tag::Item) => {
                flush(&mut lines, &mut current);
            }

            // ── Inline closers ────────────────────────────────────────────────

            Event::End(Tag::Emphasis)
            | Event::End(Tag::Strong)
            | Event::End(Tag::Strikethrough)
            | Event::End(Tag::Link(_, _, _)) => {
                style_stack.pop();
            }

            Event::End(Tag::Image(_, _, _)) => {
                in_image = false;
            }

            // ── Leaf events ───────────────────────────────────────────────────

            Event::Text(text) => {
                if in_image {
                    // The alt-text is already represented by the "[image]" span.
                    continue;
                }
                let style = top(&style_stack);
                let owned = text.into_string();
                if in_code_block {
                    // Code blocks can contain embedded newlines; split them so
                    // each becomes a separate ratatui Line.
                    let mut first = true;
                    for line_str in owned.lines() {
                        if !first {
                            flush(&mut lines, &mut current);
                        }
                        first = false;
                        current.push(Span::styled(line_str.to_string(), style));
                    }
                    if owned.ends_with('\n') {
                        flush(&mut lines, &mut current);
                    }
                } else {
                    current.push(Span::styled(owned, style));
                }
            }

            Event::Code(text) => {
                // Inline code — always green, regardless of nesting style.
                current.push(Span::styled(
                    text.into_string(),
                    Style::default().fg(Color::Green),
                ));
            }

            // SoftBreak is a line-wrapping hint inside a paragraph; treat it
            // as a single space so words don't run together.
            Event::SoftBreak => {
                current.push(Span::raw(" "));
            }

            Event::HardBreak => {
                flush(&mut lines, &mut current);
            }

            Event::Rule => {
                flush(&mut lines, &mut current);
                lines.push(hr_line());
                lines.push(Line::from(""));
            }

            // Anything else (footnotes, task-list markers, raw HTML pass-
            // through, etc.) is silently ignored — RSS content rarely uses
            // these Markdown extensions.
            _ => {}
        }
    }

    // Flush any trailing inline content not terminated by a block-end event.
    if !current.is_empty() {
        lines.push(Line::from(current));
    }

    Text::from(lines)
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Tracks whether a list level is ordered (with a counter) or unordered.
enum ListKind {
    Unordered,
    Ordered(u64),
}

/// Returns the [`Style`] at the top of `stack`, defaulting to `Style::default()`.
#[inline]
fn top(stack: &[Style]) -> Style {
    stack.last().copied().unwrap_or_default()
}

/// Moves `spans` into a new [`Line`] appended to `lines`, leaving `spans` empty.
#[inline]
fn flush(lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>) {
    lines.push(Line::from(std::mem::take(spans)));
}

/// Returns a dim horizontal-rule [`Line`].
fn hr_line() -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(40),
        Style::default().fg(Color::DarkGray),
    ))
}

/// Maps a Markdown heading level to a fully-specified ratatui [`Style`].
fn heading_style(level: HeadingLevel) -> Style {
    match level {
        HeadingLevel::H1 => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        HeadingLevel::H2 => Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
        HeadingLevel::H3 => Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        _ => Style::default().add_modifier(Modifier::BOLD),
    }
}
