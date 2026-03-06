# Cross-Platform Browser Open

**Date:** 2026-03-06T00:02:00

## Summary

Replaced the manual `#[cfg(target_os)]` browser-launch code in `open_in_browser`
with the [`open`](https://crates.io/crates/open) crate (v5), which handles all
three major platforms transparently:

| Platform | Delegate command |
|----------|-----------------|
| Linux    | `xdg-open`      |
| macOS    | `open`          |
| Windows  | `start`         |

On Linux the crate also detects WSL and Docker environments and adjusts
accordingly (via `is-wsl` and `is-docker` transitive dependencies).

## Changes

- `Cargo.toml` — added `open = "5"` dependency.
- `src/app.rs` — `open_in_browser` now:
  - Accepts `&mut self` (was `&self`) to allow setting status bar feedback.
  - Calls `open::that(&url)` instead of platform-gated `std::process::Command`.
  - Sets an `Info` status message on success, showing the opened URL.
  - Sets an `Error` status message if the OS cannot find a handler.
  - Shows an `Info` message when the selected article has no link.

## Keybinding

`o` in normal mode — unchanged.
