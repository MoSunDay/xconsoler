//! Visual theme, ported from terminator-rust's default builtin theme
//! (Kanagawa Wave) for an opaque launcher bar.
//!
//! Source of truth: terminator-rust `crates/theme/src/builtin.rs`
//! `kanagawa_wave()`. The bar is opaque: the host terminal profile paints the
//! background (see `scripts/xc-bar`, which pins the same `#1f1f28`), so the
//! launcher paints no backgrounds of its own and text, cursor and selection
//! ink stay readable.

use ratatui::style::Color;

/// Theme background: kanagawa `sumiInk0` #1f1f28 (terminator-rust palette
/// `background`). Kept UNPAINTED - reference only: the host terminal profile
/// (matched in `scripts/xc-bar`) paints every cell, so the bar stays opaque
/// and the palette keeps a single source of truth.
pub const BG: Color = rgb(0x1f, 0x1f, 0x28);

/// Primary text: kanagawa `fujiWhite` #dcdcdc (terminator-rust palette
/// `foreground`) - user input, history inputs.
pub const TEXT: Color = rgb(0xdc, 0xdc, 0xdc);

/// Accent: kanagawa `crystalBlue` #7e9cd8 (normal slot 4) - prompt, titles,
/// alias labels.
pub const ACCENT: Color = rgb(0x7e, 0x9c, 0xd8);

/// Selected-row background: kanagawa `waveBlue2` #2d4f67 - terminator-rust
/// palette `selection_background`.
pub const SELECT_BG: Color = rgb(0x2d, 0x4f, 0x67);

/// Input cursor block: kanagawa `oldWhite` #c8c093 - terminator-rust
/// palette `cursor`.
pub const CURSOR: Color = rgb(0xc8, 0xc0, 0x93);

/// Success status: kanagawa `autumnGreen` #76946a (normal slot 2).
pub const OK: Color = rgb(0x76, 0x94, 0x6a);

/// Error status: kanagawa `samuraiRed` #c34043 (normal slot 1).
pub const ERR: Color = rgb(0xc3, 0x40, 0x43);

/// Dimmed chrome/hints: kanagawa `fujiGray` #727169 (bright slot 8) -
/// maps the old ratatui `Color::DarkGray` role.
pub const MUTED: Color = rgb(0x72, 0x71, 0x69);

/// Secondary text: kanagawa `oldWhite` #c8c093 (normal slot 7) - maps the
/// old ratatui `Color::Gray` role. Coincides with [`CURSOR`] in this
/// palette, as it does upstream.
pub const SUBTLE: Color = rgb(0xc8, 0xc0, 0x93);

/// Border/divider chrome: terminator-rust `divider` =
/// mix(background, foreground, 0.13), precomputed here as a literal
/// (lerp of #1f1f28 toward #dcdcdc at 13%).
pub const BORDER: Color = rgb(0x38, 0x38, 0x3f);

/// RGB channel triple to a ratatui [`Color::Rgb`] value.
const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mix helper mirroring terminator-rust `palette::mix` (rounded lerp),
    /// used to re-derive the precomputed BORDER literal.
    fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
        let lerp = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t).round() as u8;
        [lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2])]
    }

    fn channels(c: Color) -> [u8; 3] {
        match c {
            Color::Rgb(r, g, b) => [r, g, b],
            other => panic!("expected Color::Rgb, got {other:?}"),
        }
    }

    #[test]
    fn border_matches_divider_mix() {
        let mixed = mix(channels(BG), channels(TEXT), 0.13);
        assert_eq!(channels(BORDER), mixed);
    }

    #[test]
    fn text_and_ink_stay_distinct_from_background() {
        // Opaque readability policy: foreground-derived inks must differ
        // from the (unpainted) background reference.
        assert_ne!(channels(TEXT), channels(BG));
        assert_ne!(channels(SELECT_BG), channels(BG));
        assert_ne!(channels(CURSOR), channels(BG));
    }
}
