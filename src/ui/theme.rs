//! Theme palette · severity thresholds · color logic — single source of truth.
//! Color = severity/identity. Themes: 0 default · 1 high-contrast · 2 colorblind-friendly.

use crate::collect::AccelKind;
use ratatui::style::Color;

// ── Palette (per theme) ─────────────────────────────────
// Color = severity/identity. Themes: 0 default · 1 high-contrast · 2 colorblind-friendly (blue/orange range)
fn th() -> usize {
    crate::app::theme()
}
// Theme 3 "soft" = Catppuccin Mocha palette (widely proven low-saturation, easy-on-the-eyes palette). For truecolor terminals.
#[allow(non_snake_case)]
pub(crate) fn C_OK() -> Color {
    match th() {
        1 => Color::LightGreen,
        2 => Color::Rgb(0, 114, 178),
        3 => Color::Rgb(166, 227, 161),
        _ => Color::Green,
    }
}
#[allow(non_snake_case)]
pub(crate) fn C_WARN() -> Color {
    match th() {
        1 => Color::LightYellow,
        2 => Color::Rgb(230, 159, 0),
        3 => Color::Rgb(249, 226, 175),
        _ => Color::Yellow,
    }
}
#[allow(non_snake_case)]
pub(crate) fn C_BAD() -> Color {
    match th() {
        1 => Color::LightRed,
        2 => Color::Rgb(213, 94, 0),
        3 => Color::Rgb(243, 139, 168),
        _ => Color::Red,
    }
}
#[allow(non_snake_case)]
pub(crate) fn C_DIM() -> Color {
    match th() {
        1 => Color::Gray,
        3 => Color::Rgb(127, 132, 156),
        _ => Color::DarkGray,
    }
}
#[allow(non_snake_case)]
pub(crate) fn C_TRACK() -> Color {
    match th() {
        3 => Color::Rgb(69, 71, 90),
        _ => Color::Indexed(236),
    }
}
#[allow(non_snake_case)]
pub(crate) fn C_HEAD() -> Color {
    match th() {
        1 => Color::White,
        3 => Color::Rgb(166, 173, 200),
        _ => Color::Indexed(244),
    }
}
#[allow(non_snake_case)]
pub(crate) fn C_ACC() -> Color {
    match th() {
        1 => Color::LightCyan,
        2 => Color::Rgb(86, 180, 233),
        3 => Color::Rgb(137, 220, 235),
        _ => Color::Cyan,
    }
}
#[allow(non_snake_case)]
pub(crate) fn C_HL() -> Color {
    match th() {
        3 => Color::Rgb(49, 50, 68),
        _ => Color::Indexed(238),
    }
}

pub(crate) const FRAC: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

// ── Severity thresholds (single source of truth) ────────
// Define the "warn/bad" concept per metric in one place → bar color and value color never diverge.
pub(crate) const UTIL_WARN: f64 = 60.0;
pub(crate) const UTIL_BAD: f64 = 85.0;
pub(crate) const MEM_WARN: f64 = 70.0;
pub(crate) const MEM_BAD: f64 = 90.0;
pub(crate) const TEMP_WARN: f64 = 60.0;
pub(crate) const TEMP_BAD: f64 = 80.0;
pub(crate) const IDLE_UTIL: f64 = 10.0; // below this is dim (idle)

/// Color by cell position (green→yellow→red). Same basis as the util thresholds.
pub(crate) fn grad_color(pct: f64) -> Color {
    if pct > UTIL_BAD {
        C_BAD()
    } else if pct > UTIL_WARN {
        C_WARN()
    } else {
        C_OK()
    }
}
pub(crate) fn util_color(p: f64) -> Color {
    if p > UTIL_BAD {
        C_BAD()
    } else if p > UTIL_WARN {
        C_WARN()
    } else if p > IDLE_UTIL {
        C_OK()
    } else {
        C_DIM()
    }
}
pub(crate) fn mem_color(p: f64) -> Color {
    if p > MEM_BAD {
        C_BAD()
    } else if p > MEM_WARN {
        C_WARN()
    } else if p > 1.0 {
        C_OK()
    } else {
        C_DIM()
    }
}
pub(crate) fn temp_color(t: f64) -> Color {
    if t > TEMP_BAD {
        C_BAD()
    } else if t > TEMP_WARN {
        C_WARN()
    } else if t > 0.0 {
        Color::Gray
    } else {
        C_DIM()
    }
}
pub(crate) fn kind_color(k: AccelKind) -> Color {
    if th() == 3 {
        // Catppuccin: green / mauve / sky — harmonize vendor identity colors with the palette.
        return match k {
            AccelKind::Gpu => Color::Rgb(166, 227, 161),
            AccelKind::Rbln => Color::Rgb(203, 166, 247),
            AccelKind::Rngd => Color::Rgb(137, 220, 235),
        };
    }
    match k {
        AccelKind::Gpu => Color::Green,
        AccelKind::Rbln => Color::Magenta,
        AccelKind::Rngd => Color::Cyan,
    }
}
/// prefill/decode phase colors — theme/colorblind aware (Okabe-Ito range). For distinguishing phases in the Perf table.
#[allow(non_snake_case)]
pub(crate) fn C_PREFILL() -> Color {
    match th() {
        1 => Color::LightCyan,
        2 => Color::Rgb(86, 180, 233),
        3 => Color::Rgb(137, 180, 250),
        _ => Color::Cyan,
    }
}
#[allow(non_snake_case)]
pub(crate) fn C_DECODE() -> Color {
    match th() {
        1 => Color::LightMagenta,
        2 => Color::Rgb(204, 121, 167),
        3 => Color::Rgb(203, 166, 247),
        _ => Color::Magenta,
    }
}

pub(crate) const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
