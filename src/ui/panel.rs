//! Panel system — a thin abstraction unifying views as "a layout of titled panels".
//! Replaces hand-written Layout constraints per view with a Dashboard builder → consistent layout & chrome.
//! Layout is responsive tiles (tile_rects) or explicit row/col spans. Render: each panel's closure gets an inner Rect.

use super::widgets::tile_rects;
use ratatui::layout::Rect;
use ratatui::Frame;

type RenderFn<'a> = Box<dyn FnOnce(&mut Frame, Rect) + 'a>;

/// Responsive tile layout of cells. Stack with `.cell()`/`.panel()` and draw with `.render()`.
/// min_w = tile minimum width (limits column count). Two kinds can be mixed:
/// · `.cell`  = passes the tile Rect as-is — for widgets that draw their own chrome (Table+scrollbar, bar_timeline, etc.).
/// · `.panel` = auto-draws a title block and passes the inner Rect — for pure content like Paragraph.
pub(crate) struct Dashboard<'a> {
    cells: Vec<RenderFn<'a>>,
    min_w: u16,
}

impl<'a> Dashboard<'a> {
    pub fn new() -> Self {
        Dashboard {
            cells: Vec::new(),
            min_w: 24,
        }
    }
    /// Tile minimum width (limits column count so tiles never get narrower than this).
    pub fn min_width(mut self, w: u16) -> Self {
        self.min_w = w;
        self
    }
    /// Cell that receives the tile Rect as-is (for widgets that draw their own border).
    pub fn cell(mut self, render: impl FnOnce(&mut Frame, Rect) + 'a) -> Self {
        self.cells.push(Box::new(render));
        self
    }
    /// Lay out as responsive tiles, then render each cell.
    pub fn render(self, f: &mut Frame, area: Rect) {
        let rects = tile_rects(area, self.cells.len(), self.min_w);
        for (rect, cell) in rects.into_iter().zip(self.cells) {
            cell(f, rect);
        }
    }
}
