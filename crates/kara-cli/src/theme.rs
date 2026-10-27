//! Color palette for the full-screen TUI.
//!
//! One place to tune the look; every other module asks `theme::` for a
//! style instead of inventing colors inline.

use ratatui::style::{Color, Modifier, Style};

// "House of El" (Supergirl's suit), shared with the desktop app: red is
// Kara's own mark, input border and calls to action; blue is what you
// typed and the live activity lines, so red never reads as an error.
pub const ACCENT: Color = Color::Rgb(0xef, 0x4b, 0x55);
pub const USER: Color = Color::Rgb(0x5b, 0x8c, 0xff);
pub const KRYPTON: Color = Color::Rgb(0x5b, 0x8c, 0xff);
pub const OK: Color = Color::Rgb(0x6b, 0xc2, 0x7a);
pub const ERR: Color = Color::Rgb(0xe0, 0x6c, 0x6c);
pub const WARN: Color = Color::Rgb(0xd9, 0xb3, 0x5a);
pub const DIM: Color = Color::Rgb(0x7a, 0x7a, 0x7a);
pub const ADD: Color = Color::Rgb(0x6b, 0xc2, 0x7a);
pub const DEL: Color = Color::Rgb(0xe0, 0x6c, 0x6c);
pub const HUNK: Color = Color::Rgb(0x8f, 0xb0, 0xd9);

pub fn accent() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}
pub fn accent_dim() -> Style {
    Style::default().fg(ACCENT)
}
/// Live activity (tool calls, the step in progress).
pub fn krypton() -> Style {
    Style::default().fg(KRYPTON)
}
pub fn user() -> Style {
    Style::default().fg(USER).add_modifier(Modifier::BOLD)
}
pub fn ok() -> Style {
    Style::default().fg(OK)
}
pub fn ok_bold() -> Style {
    Style::default().fg(OK).add_modifier(Modifier::BOLD)
}
pub fn err() -> Style {
    Style::default().fg(ERR)
}
pub fn err_bold() -> Style {
    Style::default().fg(ERR).add_modifier(Modifier::BOLD)
}
pub fn warn() -> Style {
    Style::default().fg(WARN)
}
pub fn dim() -> Style {
    Style::default().fg(DIM)
}
pub fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}
pub fn add() -> Style {
    Style::default().fg(ADD)
}
pub fn del() -> Style {
    Style::default().fg(DEL)
}
pub fn hunk() -> Style {
    Style::default().fg(HUNK)
}

/// Same accent color as `accent()`/`ACCENT` above, for the plain-terminal
/// (`console` crate) output paths: permission prompts and slash commands
/// that drop out of the full-screen UI. One color, two rendering libraries.
pub fn console_accent() -> console::Style {
    console::Style::new().color256(ACCENT_256).bold()
}

/// Nearest xterm-256 color to `ACCENT`, for the `console` crate paths.
pub const ACCENT_256: u8 = 203;

/// `KRYPTON` for the `console` crate paths (xterm-256 #69 ≈ #5f87ff).
pub fn console_krypton() -> console::Style {
    console::Style::new().color256(69)
}
