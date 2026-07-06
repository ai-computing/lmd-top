//! Command palette — open with `:` to fuzzy-search view/display actions by name and run them.
//! k9s's `:command` discoverability + gitui's fuzzy_find pattern (score + match indices) in pure Rust.
//! No external crates: the candidate set is small (views + display toggles, ~15), so a subsequence scorer suffices.
//!
//! Safety: the palette exposes **navigation/display actions only** (view switch, theme, alerts, pause…).
//! Cluster mutations like scale/delete stay in the existing action menu (permission gate + confirm) — no palette bypass.

use crate::app::View;

/// Actions the palette can run (observe/display only, no cluster mutation).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PaletteAction {
    Goto(View),  // jump to view (including hub sub-views)
    Help,        // help overlay
    Alerts,      // alert history
    Theme,       // cycle theme
    Pause,       // toggle update pause
    Zoom,        // toggle focus (zoom)
    Grafana,     // open Grafana in browser
    ResetEnergy, // reset the session energy baseline
}

/// A single palette entry — display label + extra hint + action to run.
#[derive(Clone)]
pub struct PaletteItem {
    pub label: String,
    pub hint: &'static str, // dim description on the right (e.g. "view", "display")
    pub action: PaletteAction,
}

/// Command palette state — query, filtered indices (by score), cursor.
pub struct Palette {
    pub query: String,
    items: Vec<PaletteItem>,
    /// (items index, matched char indices) — descending by score. When query is empty, all items (original order).
    pub filtered: Vec<(usize, Vec<usize>)>,
    pub cursor: usize,
}

impl Palette {
    /// Global palette (available from any view) — view jumps + display actions.
    pub fn global() -> Self {
        use PaletteAction::*;
        let v = |view: View| PaletteItem {
            label: view.title().to_string(),
            hint: "view",
            action: Goto(view),
        };
        let mut items = vec![
            v(View::Overview),
            v(View::Nodes),
            v(View::Accel),
            v(View::Perf),
            v(View::Topo),
            v(View::Epp),
            v(View::Routing),
            v(View::Pods),
            v(View::Serving),
            v(View::Library),
            v(View::Zoo),
            v(View::Events),
            v(View::Setup),
        ];
        items.extend([
            PaletteItem {
                label: "Theme".into(),
                hint: "cycle theme",
                action: Theme,
            },
            PaletteItem {
                label: "Alerts".into(),
                hint: "alert history",
                action: Alerts,
            },
            PaletteItem {
                label: "Pause".into(),
                hint: "freeze updates",
                action: Pause,
            },
            PaletteItem {
                label: "Zoom".into(),
                hint: "focus mode",
                action: Zoom,
            },
            PaletteItem {
                label: "Grafana".into(),
                hint: "open in browser",
                action: Grafana,
            },
            PaletteItem {
                label: "Reset energy".into(),
                hint: "session baseline",
                action: ResetEnergy,
            },
            PaletteItem {
                label: "Help".into(),
                hint: "keybindings",
                action: Help,
            },
        ]);
        let mut p = Palette {
            query: String::new(),
            items,
            filtered: Vec::new(),
            cursor: 0,
        };
        p.refilter();
        p
    }

    /// Re-filter after a query change — matches only, descending by score (ties keep original order, stable).
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
            // Descending by score; ties broken by ascending original index (stable).
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
    /// Item the cursor currently points at (with label and match indices) — shared by render/run.
    pub fn selected(&self) -> Option<PaletteAction> {
        self.filtered
            .get(self.cursor)
            .map(|(i, _)| self.items[*i].action)
    }
    /// For rendering — list of (label, hint, matched char indices, selected).
    pub fn rows(&self) -> Vec<(&str, &'static str, &[usize], bool)> {
        self.filtered
            .iter()
            .enumerate()
            .map(|(row, (i, idx))| {
                let it = &self.items[*i];
                (
                    it.label.as_str(),
                    it.hint,
                    idx.as_slice(),
                    row == self.cursor,
                )
            })
            .collect()
    }
}

/// Subsequence fuzzy match — if `needle` is an (order-preserving) subsequence of `hay`, Some((score, matched char indices)).
/// Score: base per match + contiguous-match bonus + word-boundary (start / after separator) bonus − gap penalty.
/// Case-insensitive. With a small candidate set, one greedy left→right pass suffices (not an optimal alignment).
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
        // Find the next match position starting from hi.
        let found = (hi..hchars.len()).find(|&j| hchars[j].to_ascii_lowercase() == nc_l);
        let j = found?; // if any char can't be found, not a subsequence → reject
        score += 8; // base per match
        let at_start = j == 0;
        let after_sep = j > 0 && matches!(hchars[j - 1], ' ' | '-' | '_' | ':' | '/' | '.');
        if at_start || after_sep {
            score += 12; // word-boundary match bonus
        }
        match prev_match {
            Some(p) if j == p + 1 => score += 10, // contiguous with previous match → bonus
            Some(p) => score -= (j - p - 1).min(6) as i32, // gap penalty (capped)
            None => {}
        }
        matched.push(j);
        prev_match = Some(j);
        hi = j + 1;
    }
    // Slightly favor shorter hay (closer to exact match) — length-difference penalty.
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
        // For "ep", "EPP" (prefix + contiguous) should outrank "Deploy" (scattered).
        let epp = fuzzy_match("EPP", "ep").unwrap().0;
        let deploy = fuzzy_match("Deploy", "ep").unwrap().0;
        assert!(
            epp > deploy,
            "EPP({}) should outrank Deploy({})",
            epp,
            deploy
        );
    }

    #[test]
    fn palette_filters_and_orders() {
        let mut p = Palette::global();
        assert!(!p.filtered.is_empty());
        let full = p.filtered.len();
        for c in "epp".chars() {
            p.push(c);
        }
        // Top result should be the EPP view.
        assert_eq!(p.selected(), Some(PaletteAction::Goto(View::Epp)));
        assert!(p.filtered.len() < full);
        // Restore via backspace.
        p.pop();
        p.pop();
        p.pop();
        assert_eq!(p.filtered.len(), full);
    }

    #[test]
    fn cursor_wraps() {
        let mut p = Palette::global();
        p.move_cursor(-1); // up → wraps to last
        assert_eq!(p.cursor, p.filtered.len() - 1);
        p.move_cursor(1); // down → back to first
        assert_eq!(p.cursor, 0);
    }
}
