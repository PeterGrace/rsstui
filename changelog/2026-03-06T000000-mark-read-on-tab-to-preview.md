# Fix: Mark Article as Read When Tabbing into Preview Pane

**Date:** 2026-03-06

## Problem

Switching to the Preview pane via `Tab` or `Shift+Tab` did not mark the currently selected article as read, even though pressing `Enter` on an article (which also opens the Preview pane) correctly called `mark_read(true)`. This was an inconsistency in the read-state logic.

## Change

`src/app.rs` — `handle_key_normal`: Refactored the `Tab` and `BackTab` key handlers to compute the destination pane first, then call `mark_read(true)` whenever the destination is `ActivePane::Preview`, before updating `self.active_pane`.

```rust
// Before (pseudocode):
self.active_pane = match self.active_pane { ... };  // no read marking

// After:
let next = match self.active_pane { ... };
if next == ActivePane::Preview {
    self.mark_read(true);
}
self.active_pane = next;
```

## Mark as Unread

Already supported via the `m` key, which toggles read state in all panes (including Preview). No change needed.

## Testing

- `cargo build` — no warnings or errors.
