//! 패널 시스템 — 뷰를 "제목 있는 패널들의 배치"로 통일하는 얇은 추상화.
//! 각 뷰가 Layout constraints 를 손으로 짜던 것을 Dashboard 빌더로 대체 → 배치·chrome 일관.
//! 배치는 반응형 타일(tile_rects) 또는 명시적 행/열 스팬. 렌더는 패널별 클로저가 inner Rect 를 받음.

use super::widgets::tile_rects;
use ratatui::layout::Rect;
use ratatui::Frame;

type RenderFn<'a> = Box<dyn FnOnce(&mut Frame, Rect) + 'a>;

/// 셀들의 반응형 타일 배치. `.cell()`/`.panel()` 로 쌓고 `.render()` 로 그림.
/// min_w = 타일 최소 폭(열 수 제한). 두 종류를 섞을 수 있음:
/// · `.cell`  = 타일 Rect 를 그대로 넘김 — 자체 chrome 을 그리는 위젯(Table+scrollbar, bar_timeline 등)용.
/// · `.panel` = 제목 블록을 자동으로 그리고 inner Rect 를 넘김 — Paragraph 등 순수 내용용.
pub(crate) struct Dashboard<'a> {
    cells: Vec<RenderFn<'a>>,
    min_w: u16,
}

impl<'a> Dashboard<'a> {
    pub fn new() -> Self {
        Dashboard { cells: Vec::new(), min_w: 24 }
    }
    /// 타일 최소 폭(이보다 좁아지지 않게 열 수 제한).
    pub fn min_width(mut self, w: u16) -> Self {
        self.min_w = w;
        self
    }
    /// 타일 Rect 를 그대로 받는 셀(자체 테두리를 그리는 위젯용).
    pub fn cell(mut self, render: impl FnOnce(&mut Frame, Rect) + 'a) -> Self {
        self.cells.push(Box::new(render));
        self
    }
    /// 반응형 타일로 배치 후 각 셀 렌더.
    pub fn render(self, f: &mut Frame, area: Rect) {
        let rects = tile_rects(area, self.cells.len(), self.min_w);
        for (rect, cell) in rects.into_iter().zip(self.cells) {
            cell(f, rect);
        }
    }
}
