# Clamp Preview Pane Scroll to Content Bounds

**Date:** 2026-03-06

## Problem

The preview pane allowed the user to scroll down indefinitely past the end of the article content, resulting in a blank viewport with no feedback.

## Solution

Track the total wrapped line count and visible area height across render frames, then clamp all downward scroll increments to `content_lines - area_height`.

### Changes

#### `src/app.rs`
- Added `preview_content_lines: u16` — total wrapped row count of the current article (updated each render frame).
- Added `preview_area_height: u16` — inner height of the preview block in terminal rows (updated each render frame).
- Added `App::preview_max_scroll()` helper returning `preview_content_lines.saturating_sub(preview_area_height)`.
- All `preview_scroll` increment sites (`j` / `d` keys, `navigate_down`) now call `.min(self.preview_max_scroll())` after `saturating_add`.

#### `src/ui.rs`
- Changed `render_preview_pane` signature from `&App` to `&mut App` to allow writing the two new fields.
- After building the `Text` from `render_markdown`, iterates each `Line`, computes wrapped row count via `line.width().div_ceil(inner_width)`, and stores the sum in `app.preview_content_lines`.
- Stores `area.height.saturating_sub(2)` (inner height minus borders) in `app.preview_area_height`.
