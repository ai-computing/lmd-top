//! tachyonfx 기반 화면 이펙트(순수 Rust) — 뷰 전환 트랜지션 + 신규 알림 플래시.
//! 상태(진행 중 Effect)는 프레임 간 유지돼야 하므로 App(데이터 모델)이 아니라 UI 루프가 소유.
//! 이펙트는 이미 그려진 버퍼를 후처리(post-process)한다 → 위젯 렌더 뒤에 area 위로 적용.

use super::theme::*;
use crate::app::{App, View};
use ratatui::layout::Rect;
use ratatui::Frame;
use std::time::Instant;
use tachyonfx::{fx, Duration as FxDuration, Effect, EffectRenderer, Interpolation, Shader};

pub struct FxState {
    enabled: bool,
    last: Instant,
    prev_view: Option<View>,
    prev_detail: bool,
    prev_flash: u64,
    body: Option<Effect>,  // 뷰 전환/상세 진입 트랜지션(본문 영역)
    flash: Option<Effect>, // 신규 알림 플래시(요약 바)
}

impl FxState {
    pub fn new() -> Self {
        FxState { enabled: true, last: Instant::now(), prev_view: None, prev_detail: false, prev_flash: 0, body: None, flash: None }
    }
    /// 이펙트 비활성(--render 텍스트 덤프 등, 부분 프레임이 결과를 오염시키지 않게).
    pub fn disabled() -> Self {
        FxState { enabled: false, ..Self::new() }
    }

    /// 프레임 시작: 경과시간(dt) 산출 + 상태변화 감지해 이펙트 무장. dt 는 render_effect 에 넘김.
    pub fn begin(&mut self, app: &App) -> FxDuration {
        let now = Instant::now();
        // 첫 프레임/일시정지 후 큰 점프는 캡(이펙트가 순간이동하지 않게).
        let dt: FxDuration = now.duration_since(self.last).min(std::time::Duration::from_millis(100)).into();
        self.last = now;
        if !self.enabled {
            return dt;
        }
        // 은은하게, 상황별로 다른 느낌을 섞음(테마-안전: 전경색만 건드림 → 밝은/어두운 터미널 무관).
        // · 뷰 전환 = 텍스트가 dim 에서 원색으로 부드럽게 "잉크인"(방향/스캐터 없음 → 눈 안 아픔).
        // · 상세 진입 = 살짝 coalesce(줌인 느낌) 짧게 — 전환과 구분되는 결.
        let view_changed = self.prev_view.map(|v| v != app.view).unwrap_or(true);
        let detail_entered = app.detail && !self.prev_detail;
        if detail_entered {
            self.body = Some(fx::coalesce((180u32, Interpolation::QuadOut)));
        } else if view_changed {
            self.body = Some(fx::fade_from_fg(C_DIM(), (150u32, Interpolation::QuadOut)));
        }
        self.prev_view = Some(app.view);
        self.prev_detail = app.detail;
        // 신규 알림 → 요약 바 전경색이 잠깐 빨개졌다 원색으로(배경 전체 빨강 아님 → 덜 자극적).
        if app.flash_until > self.prev_flash {
            self.flash = Some(fx::fade_from_fg(C_BAD(), (450u32, Interpolation::QuadOut)));
        }
        self.prev_flash = app.flash_until;
        dt
    }

    /// 본문 트랜지션 적용(위젯 렌더 직후, 오버레이 전에).
    pub fn body(&mut self, f: &mut Frame, area: Rect, dt: FxDuration) {
        if let Some(e) = self.body.as_mut() {
            f.render_effect(e, area, dt);
            if e.done() {
                self.body = None;
            }
        }
    }

    /// 요약 바 플래시 적용.
    pub fn flash(&mut self, f: &mut Frame, area: Rect, dt: FxDuration) {
        if let Some(e) = self.flash.as_mut() {
            f.render_effect(e, area, dt);
            if e.done() {
                self.flash = None;
            }
        }
    }

    /// 진행 중 이펙트가 있으면 true — 애니메이션 동안 렌더 루프를 빠르게 돌리기 위한 힌트.
    pub fn animating(&self) -> bool {
        self.body.is_some() || self.flash.is_some()
    }
}
