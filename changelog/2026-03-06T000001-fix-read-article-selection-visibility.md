# Fix: Read Article Selection Visibility

**Date:** 2026-03-06

## Problem

When navigating the articles pane, selecting a read article made it invisible. The
highlight style used `bg(Color::DarkGray)`, which is identical to the foreground color
of `READ_STYLE` (`fg(Color::DarkGray)`). Dark gray text on a dark gray background
produced no visible contrast.

## Fix

Changed the articles pane `highlight_style` in `src/ui.rs` from:

```rust
Style::default()
    .bg(Color::DarkGray)
    .add_modifier(Modifier::BOLD)
```

to:

```rust
Style::default()
    .bg(Color::Blue)
    .fg(Color::White)
    .add_modifier(Modifier::BOLD)
```

The explicit `fg(Color::White)` overrides both `READ_STYLE` (DarkGray) and
`UNREAD_STYLE` (White) for the selected row, ensuring the highlighted article is
always legible regardless of its read/unread state.

## Files Changed

- `src/ui.rs`: `render_articles_pane` highlight style updated.
