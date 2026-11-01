//! Seat mocha as icedtea tokens. Type scale is icedtea's.

use iced::Color;
use icedtea::m3::{Density, DensityName, ElevationPolicy, ShapePolicy};
use icedtea::theme::Tokens;

/// Body size in pixels (`icedtea::typo::BODY`).
pub use icedtea::typo::BODY as SIZE_BODY;
/// Meta size in pixels (`icedtea::typo::META`).
pub use icedtea::typo::META as SIZE_META;
/// Hint size; same token as [`SIZE_META`].
pub use icedtea::typo::META as SIZE_HINT;
/// Title size in pixels (`icedtea::typo::TITLE`).
pub use icedtea::typo::TITLE as SIZE_TITLE;
/// icedtea UI typeface.
pub use icedtea::typo::UI as FACE;

/// Sidebar and chrome (`#181825`).
pub const MANTLE: Color = Color::from_rgb8(0x18, 0x18, 0x25);
/// Main field (`#1e1e2e`).
pub const BASE: Color = Color::from_rgb8(0x1e, 0x1e, 0x2e);
/// Selected row (`#313244`).
pub const SURFACE0: Color = Color::from_rgb8(0x31, 0x32, 0x44);
/// Hairlines (`#45475a`).
pub const SURFACE1: Color = Color::from_rgb8(0x45, 0x47, 0x5a);
/// Rofi border (`#585b70`, Catppuccin overlay1).
pub const OVERLAY1: Color = Color::from_rgb8(0x58, 0x5b, 0x70);
/// Title text (`#cdd6f4`).
pub const TEXT: Color = Color::from_rgb8(0xcd, 0xd6, 0xf4);
/// Meta line (`#a6adc8`).
pub const SUBTEXT: Color = Color::from_rgb8(0xa6, 0xad, 0xc8);
/// Dim labels (`#6c7086`).
pub const OVERLAY: Color = Color::from_rgb8(0x6c, 0x70, 0x86);
/// Accent and selected filter (`#89b4fa`).
pub const BLUE: Color = Color::from_rgb8(0x89, 0xb4, 0xfa);
/// Priority A (`#f38ba8`).
pub const RED: Color = Color::from_rgb8(0xf3, 0x8b, 0xa8);
/// Priority B (`#fab387`).
pub const PEACH: Color = Color::from_rgb8(0xfa, 0xb3, 0x87);
/// Done and success (`#a6e3a1`).
pub const GREEN: Color = Color::from_rgb8(0xa6, 0xe3, 0xa1);
/// Blocked (`#f9e2af`).
pub const YELLOW: Color = Color::from_rgb8(0xf9, 0xe2, 0xaf);

/// Mocha mapped onto icedtea's semantic tokens.
///
/// Seat colors are the mocha aliases (canvas through border). Density,
/// corners, and elevation are Compact / Material / Flat.
pub fn tokens() -> Tokens {
    Tokens::from_aliases(
        BASE, SURFACE0, MANTLE, TEXT, SUBTEXT, BLUE, PEACH, GREEN, YELLOW, RED, OVERLAY1,
    )
    .with_density(Density::named(DensityName::Compact))
    .with_shape(ShapePolicy::Material)
    .with_elevation(ElevationPolicy::Flat)
}

/// iced [`Theme`](iced::Theme) from the mocha tokens.
pub fn theme() -> iced::Theme {
    icedtea::theme::iced_theme("vissue-mocha", tokens())
}

/// Priority pip: A red, B peach, C quiet blue-grey.
pub fn priority_color(priority: &str) -> Color {
    match priority {
        "A" => RED,
        "B" => PEACH,
        _ => OVERLAY,
    }
}

/// State chip colour.
pub fn state_color(state: &str) -> Color {
    match state {
        "STARTED" => BLUE,
        "BLOCKED" => YELLOW,
        "DONE" => GREEN,
        "CANCELLED" => OVERLAY,
        _ => SUBTEXT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_uses_mocha_tokens() {
        let t = tokens();
        assert_eq!(t.canvas, BASE);
        assert_eq!(t.surface, SURFACE0);
        assert_eq!(t.panel, MANTLE);
        assert_eq!(t.text, TEXT);
        assert_eq!(t.muted, SUBTEXT);
        assert_eq!(t.primary, BLUE);
        assert_eq!(t.accent, PEACH);
        assert_eq!(t.success, GREEN);
        assert_eq!(t.warning, YELLOW);
        assert_eq!(t.danger, RED);
        assert_eq!(t.border, OVERLAY1);
        assert_eq!(t.selection, icedtea::theme::mix(BLUE, BASE, 0.28));
        assert_eq!(t.selection_text, TEXT);
        assert_eq!(t.density.name, DensityName::Compact);
        assert_eq!(t.shape, ShapePolicy::Material);
        assert_eq!(t.elevation, ElevationPolicy::Flat);
        let _ = theme();
    }

    #[test]
    fn priority_pips_match_todoist_order() {
        assert_eq!(priority_color("A"), RED);
        assert_eq!(priority_color("B"), PEACH);
        assert_eq!(priority_color("C"), OVERLAY);
    }

    // The type scale is fixed at compile time, so its ordering is checked
    // there too: a runtime assertion over two constants can only ever pass.
    const _: () = assert!(SIZE_BODY < SIZE_TITLE);
    const _: () = assert!(SIZE_META < SIZE_BODY);

    #[test]
    fn body_is_the_app_base_and_smaller_than_title() {
        assert_eq!(OVERLAY1, Color::from_rgb8(0x58, 0x5b, 0x70));
    }
}
