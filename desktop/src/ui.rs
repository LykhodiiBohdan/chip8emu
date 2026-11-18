//! Small immediate-mode UI toolkit drawn with SDL2 primitives.
//!
//! The desktop host deliberately avoids extra GUI dependencies: the emulator
//! already links SDL2, and an 8x8 bitmap font plus `fill_rect` is enough for a
//! settings sheet that matches the web page.

use crate::font;
use crate::settings::Rgb;
use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::render::Canvas;
use sdl2::video::Window;

pub const TEXT: Color = Color::RGB(232, 232, 234);
pub const MUTED: Color = Color::RGB(139, 139, 147);
pub const ACCENT: Color = Color::RGB(0, 213, 224);
pub const FIELD: Color = Color::RGB(35, 35, 40);
pub const FIELD_HOVER: Color = Color::RGB(48, 48, 56);
pub const FIELD_ACTIVE: Color = Color::RGB(58, 58, 68);
pub const BORDER: Color = Color::RGB(52, 52, 60);
pub const PANEL: Color = Color::RGB(26, 26, 30);
pub const PANEL_BORDER: Color = Color::RGB(70, 70, 82);
pub const DIMMER: Color = Color::RGBA(8, 8, 10, 190);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Normal,
    Selected,
    Accent,
}

pub struct Ui<'a> {
    canvas: &'a mut Canvas<Window>,
    mouse: (i32, i32),
    pressed: bool,
    clicked: bool,
}

impl<'a> Ui<'a> {
    pub fn new(
        canvas: &'a mut Canvas<Window>,
        mouse: (i32, i32),
        pressed: bool,
        clicked: bool,
    ) -> Self {
        Self {
            canvas,
            mouse,
            pressed,
            clicked,
        }
    }

    pub fn fill(&mut self, rect: Rect, color: Color) {
        self.canvas.set_draw_color(color);
        let _ = self.canvas.fill_rect(rect);
    }

    pub fn outline(&mut self, rect: Rect, thickness: u32, color: Color) {
        let t = thickness as i32;
        self.fill(Rect::new(rect.x(), rect.y(), rect.width(), t as u32), color);
        self.fill(
            Rect::new(
                rect.x(),
                rect.y() + rect.height() as i32 - t,
                rect.width(),
                t as u32,
            ),
            color,
        );
        self.fill(
            Rect::new(rect.x(), rect.y(), t as u32, rect.height()),
            color,
        );
        self.fill(
            Rect::new(
                rect.x() + rect.width() as i32 - t,
                rect.y(),
                t as u32,
                rect.height(),
            ),
            color,
        );
    }

    pub fn text(&mut self, x: i32, y: i32, scale: u32, color: Color, text: &str) {
        self.canvas.set_draw_color(color);
        let mut pen = x;
        for ch in text.chars() {
            let glyph = font::glyph(ch);
            for (row, bits) in glyph.iter().enumerate() {
                let mut col = 0u32;
                while col < font::GLYPH_WIDTH {
                    if bits & (1 << col) == 0 {
                        col += 1;
                        continue;
                    }
                    // Merge horizontal runs into one rectangle per row.
                    let start = col;
                    while col < font::GLYPH_WIDTH && bits & (1 << col) != 0 {
                        col += 1;
                    }
                    let _ = self.canvas.fill_rect(Rect::new(
                        pen + start as i32 * scale as i32,
                        y + row as i32 * scale as i32,
                        (col - start) * scale,
                        scale,
                    ));
                }
            }
            pen += (font::GLYPH_WIDTH * scale) as i32;
        }
    }

    pub fn text_width(text: &str, scale: u32) -> i32 {
        (text.chars().count() as u32 * font::GLYPH_WIDTH * scale) as i32
    }

    pub fn text_height(scale: u32) -> i32 {
        (font::GLYPH_HEIGHT * scale) as i32
    }

    pub fn centered_text(&mut self, rect: Rect, scale: u32, color: Color, text: &str) {
        let x = rect.x() + (rect.width() as i32 - Self::text_width(text, scale)) / 2;
        let y = rect.y() + (rect.height() as i32 - Self::text_height(scale)) / 2;
        self.text(x, y, scale, color, text);
    }

    pub fn hovered(&self, rect: Rect) -> bool {
        inside(rect, self.mouse)
    }

    /// Current drawable size in pixels, which follows resizes and fullscreen.
    pub fn window_size(&self) -> (u32, u32) {
        self.canvas.output_size().unwrap_or((1, 1))
    }

    /// Left-button click that started and finished on the same widget.
    fn invoked(&self, rect: Rect) -> bool {
        self.clicked && self.hovered(rect)
    }

    pub fn button(&mut self, rect: Rect, label: &str, style: Style) -> bool {
        let hovered = self.hovered(rect);
        let active = hovered && self.pressed;
        let background = match (style, active, hovered) {
            (Style::Selected, _, _) => Color::RGB(22, 72, 78),
            (Style::Accent, _, _) => Color::RGB(0, 92, 100),
            (_, true, _) => FIELD_ACTIVE,
            (_, false, true) => FIELD_HOVER,
            _ => FIELD,
        };
        self.fill(rect, background);

        let border = match style {
            Style::Selected | Style::Accent => ACCENT,
            _ if hovered => Color::RGB(84, 84, 96),
            _ => BORDER,
        };
        self.outline(rect, 1, border);

        let color = match style {
            Style::Selected | Style::Accent => ACCENT,
            _ => TEXT,
        };
        self.centered_text(rect, 2, color, label);
        self.invoked(rect)
    }

    pub fn chip(&mut self, rect: Rect, label: &str, selected: bool) -> bool {
        let style = if selected {
            Style::Selected
        } else {
            Style::Normal
        };
        self.button(rect, label, style)
    }

    pub fn toggle(&mut self, rect: Rect, label: &str, on: bool) -> bool {
        self.button(rect, label, if on { Style::Accent } else { Style::Normal })
    }

    /// A colour swatch that acts as a selectable tab.
    pub fn swatch(&mut self, rect: Rect, label: &str, color: Rgb, selected: bool) -> bool {
        self.fill(rect, Color::RGB(color.r, color.g, color.b));
        let border = if selected {
            ACCENT
        } else if self.hovered(rect) {
            Color::RGB(190, 190, 200)
        } else {
            BORDER
        };
        self.outline(rect, if selected { 2 } else { 1 }, border);

        // Keep the label readable on light and dark swatches.
        let text_color = if color.is_light() {
            Color::RGB(16, 16, 18)
        } else {
            Color::RGB(236, 236, 240)
        };
        self.centered_text(rect, 2, text_color, label);
        self.invoked(rect)
    }

    /// Horizontal 0..=255 slider; returns true when the value changed.
    pub fn slider(&mut self, rect: Rect, value: &mut u8, color: Rgb) -> bool {
        self.fill(rect, FIELD);
        self.outline(rect, 1, BORDER);

        let mut changed = false;
        if self.pressed
            && self.hovered(Rect::new(
                rect.x(),
                rect.y() - 6,
                rect.width(),
                rect.height() + 12,
            ))
        {
            let span = (rect.width().saturating_sub(1)) as f32;
            let ratio = ((self.mouse.0 - rect.x()) as f32 / span).clamp(0.0, 1.0);
            let next = (ratio * 255.0).round() as u8;
            if next != *value {
                *value = next;
                changed = true;
            }
        }

        let inner_width = rect.width().saturating_sub(2);
        let filled = (inner_width as f32 * (*value as f32 / 255.0)).round() as u32;
        if filled > 0 {
            self.fill(
                Rect::new(rect.x() + 1, rect.y() + 1, filled, rect.height() - 2),
                Color::RGB(color.r, color.g, color.b),
            );
        }

        // Keep the knob inside the track even at 0 and 255.
        let knob_x =
            (rect.x() + 1 + filled as i32 - 4).clamp(rect.x(), rect.x() + rect.width() as i32 - 8);
        let knob = Rect::new(knob_x, rect.y() - 3, 8, rect.height() + 6);
        self.fill(knob, Color::RGB(238, 238, 242));
        changed
    }
}

fn inside(rect: Rect, (x, y): (i32, i32)) -> bool {
    x >= rect.x()
        && y >= rect.y()
        && x < rect.x() + rect.width() as i32
        && y < rect.y() + rect.height() as i32
}
