//! 커맨드 팔레트 — `:` 로 열어 뷰/표시 액션을 이름으로 퍼지 검색해 실행.
//! k9s 의 `:command` 발견성 + gitui fuzzy_find 패턴(점수+매칭 인덱스)을 순수 Rust 로.
//! 외부 크레이트 없음: 후보 집합이 작아(뷰+표시토글 ~15개) 서브시퀀스 스코어러로 충분.
//!
//! 안전성: 팔레트는 **네비게이션·표시 액션만** 노출한다(뷰 이동, 테마, 알림, 일시정지…).
//! scale/delete 같은 클러스터 변경은 기존 액션 메뉴(권한 게이트+확인)로만 — 팔레트로 우회 불가.

use crate::app::View;

/// 팔레트가 실행할 수 있는 동작(관측/표시 전용, 클러스터 무변경).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PaletteAction {
    Goto(View), // 뷰로 점프(허브 하위 뷰 포함)
    Help,       // 도움말 오버레이
    Alerts,     // 알림 히스토리
    Theme,      // 테마 순환
    Pause,      // 갱신 일시정지 토글
    Zoom,       // 포커스(줌) 토글
    Grafana,    // 브라우저로 Grafana 열기
}

/// 팔레트 항목 하나 — 표시 라벨 + 부가 힌트 + 실행 액션.
#[derive(Clone)]
pub struct PaletteItem {
    pub label: String,
    pub hint: &'static str, // 오른쪽 흐린 설명(예: "view", "display")
    pub action: PaletteAction,
}

/// 커맨드 팔레트 상태 — 쿼리, 필터된 인덱스(점수순), 커서.
pub struct Palette {
    pub query: String,
    items: Vec<PaletteItem>,
    /// (items 인덱스, 매칭 char 인덱스) — 점수 내림차순. 쿼리 비면 전체(원순서).
    pub filtered: Vec<(usize, Vec<usize>)>,
    pub cursor: usize,
}

impl Palette {
    /// 전역 팔레트(어느 뷰에서나) — 뷰 점프 + 표시 액션.
    pub fn global() -> Self {
        use PaletteAction::*;
        let v = |view: View| PaletteItem { label: view.title().to_string(), hint: "view", action: Goto(view) };
        let mut items = vec![
            v(View::Overview),
            v(View::Nodes),
            v(View::Accel),
            v(View::Perf),
            v(View::Topo),
            v(View::Models),
            v(View::Epp),
            v(View::Routing),
            v(View::Pods),
            v(View::Launch),
            v(View::Events),
        ];
        items.extend([
            PaletteItem { label: "Theme".into(), hint: "cycle theme", action: Theme },
            PaletteItem { label: "Alerts".into(), hint: "alert history", action: Alerts },
            PaletteItem { label: "Pause".into(), hint: "freeze updates", action: Pause },
            PaletteItem { label: "Zoom".into(), hint: "focus mode", action: Zoom },
            PaletteItem { label: "Grafana".into(), hint: "open in browser", action: Grafana },
            PaletteItem { label: "Help".into(), hint: "keybindings", action: Help },
        ]);
        let mut p = Palette { query: String::new(), items, filtered: Vec::new(), cursor: 0 };
        p.refilter();
        p
    }

    /// 쿼리 변경 후 재필터 — 매칭만, 점수 내림차순(동점은 원래 순서 안정 유지).
    pub fn refilter(&mut self) {
        let q = self.query.trim();
        if q.is_empty() {
            self.filtered = (0..self.items.len()).map(|i| (i, Vec::new())).collect();
        } else {
            let mut scored: Vec<(i32, usize, Vec<usize>)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, it)| fuzzy_match(&it.label, q).map(|(s, idx)| (s, i, idx)))
                .collect();
            // 점수 내림차순, 동점이면 원래 인덱스 오름차순(안정).
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            self.filtered = scored.into_iter().map(|(_, i, idx)| (i, idx)).collect();
        }
        if self.cursor >= self.filtered.len() {
            self.cursor = self.filtered.len().saturating_sub(1);
        }
    }

    pub fn push(&mut self, c: char) {
        self.query.push(c);
        self.cursor = 0;
        self.refilter();
    }
    pub fn pop(&mut self) {
        self.query.pop();
        self.cursor = 0;
        self.refilter();
    }
    pub fn move_cursor(&mut self, d: i32) {
        let n = self.filtered.len();
        if n == 0 {
            return;
        }
        let cur = self.cursor as i32 + d;
        self.cursor = cur.rem_euclid(n as i32) as usize;
    }
    /// 현재 커서가 가리키는 항목(라벨·매칭 인덱스 함께) — 렌더/실행 공용.
    pub fn selected(&self) -> Option<PaletteAction> {
        self.filtered.get(self.cursor).map(|(i, _)| self.items[*i].action)
    }
    /// 렌더용 — (라벨, 힌트, 매칭 char 인덱스, 선택 여부) 리스트.
    pub fn rows(&self) -> Vec<(&str, &'static str, &[usize], bool)> {
        self.filtered
            .iter()
            .enumerate()
            .map(|(row, (i, idx))| {
                let it = &self.items[*i];
                (it.label.as_str(), it.hint, idx.as_slice(), row == self.cursor)
            })
            .collect()
    }
}

/// 서브시퀀스 퍼지 매칭 — needle 이 hay 의 (순서 보존) 부분수열이면 Some((점수, 매칭 char 인덱스)).
/// 점수: 매칭당 기본점 + 연속 매칭 보너스 + 단어 경계(시작/구분자 뒤) 보너스 − 간격 페널티.
/// 대소문자 무시. 후보가 작아 그리디 좌→우 1패스로 충분(최적 정렬 아님).
pub fn fuzzy_match(hay: &str, needle: &str) -> Option<(i32, Vec<usize>)> {
    let needle = needle.trim();
    if needle.is_empty() {
        return Some((0, Vec::new()));
    }
    let hchars: Vec<char> = hay.chars().collect();
    let nchars: Vec<char> = needle.chars().collect();
    let mut matched = Vec::with_capacity(nchars.len());
    let mut score = 0i32;
    let mut hi = 0usize;
    let mut prev_match: Option<usize> = None;
    for &nc in &nchars {
        let nc_l = nc.to_ascii_lowercase();
        // hi 부터 다음 매칭 위치 탐색.
        let found = (hi..hchars.len()).find(|&j| hchars[j].to_ascii_lowercase() == nc_l);
        let j = found?; // 하나라도 못 찾으면 부분수열 아님 → 탈락
        score += 8; // 매칭 기본점
        let at_start = j == 0;
        let after_sep = j > 0 && matches!(hchars[j - 1], ' ' | '-' | '_' | ':' | '/' | '.');
        if at_start || after_sep {
            score += 12; // 단어 경계 매칭 보너스
        }
        match prev_match {
            Some(p) if j == p + 1 => score += 10, // 직전 매칭과 연속 → 보너스
            Some(p) => score -= (j - p - 1).min(6) as i32, // 간격 페널티(상한)
            None => {}
        }
        matched.push(j);
        prev_match = Some(j);
        hi = j + 1;
    }
    // 짧은 hay(정확에 가까운 매칭) 소폭 우대 — 길이차 페널티.
    score -= (hchars.len().saturating_sub(nchars.len()) as i32) / 4;
    Some((score, matched))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_subsequence_rejected() {
        assert!(fuzzy_match("Overview", "xyz").is_none());
        assert!(fuzzy_match("Nodes", "zz").is_none());
    }

    #[test]
    fn subsequence_matches_and_indices() {
        let (_, idx) = fuzzy_match("Overview", "ovw").unwrap();
        assert_eq!(idx, vec![0, 1, 7]); // O(0), v(1), w(7) — greedy: 'w' is last char
        let (_, idx2) = fuzzy_match("Deploy", "dpy").unwrap();
        assert_eq!(idx2, vec![0, 2, 5]); // D(0) e p(2) l o y(5)
    }

    #[test]
    fn prefix_and_contiguous_outranks_scattered() {
        // "ep" 는 "EPP"(접두·연속)가 "Deploy"(흩어짐)보다 높아야.
        let epp = fuzzy_match("EPP", "ep").unwrap().0;
        let deploy = fuzzy_match("Deploy", "ep").unwrap().0;
        assert!(epp > deploy, "EPP({}) should outrank Deploy({})", epp, deploy);
    }

    #[test]
    fn palette_filters_and_orders() {
        let mut p = Palette::global();
        assert!(!p.filtered.is_empty());
        let full = p.filtered.len();
        for c in "epp".chars() {
            p.push(c);
        }
        // 최상위는 EPP 뷰여야.
        assert_eq!(p.selected(), Some(PaletteAction::Goto(View::Epp)));
        assert!(p.filtered.len() < full);
        // 백스페이스로 복원.
        p.pop();
        p.pop();
        p.pop();
        assert_eq!(p.filtered.len(), full);
    }

    #[test]
    fn cursor_wraps() {
        let mut p = Palette::global();
        p.move_cursor(-1); // 위로 → 마지막으로 랩
        assert_eq!(p.cursor, p.filtered.len() - 1);
        p.move_cursor(1); // 아래로 → 처음
        assert_eq!(p.cursor, 0);
    }
}
