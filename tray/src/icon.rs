//! Tray icons drawn in code. Windows has no text beside a tray icon, so the
//! reading itself is the icon there; macOS shows text next to a small dot.

use crate::model::Band;
use crate::share::Trend;

pub const SIZE: u32 = 32;

#[derive(Clone, Copy)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub fn band_color(band: Option<Band>, stale: bool) -> Rgb {
    if stale {
        return Rgb(0x6b, 0x6f, 0x76);
    }
    match band {
        Some(Band::UrgentLow) => Rgb(0xc6, 0x28, 0x28),
        Some(Band::Low) => Rgb(0xe0, 0x45, 0x3a),
        Some(Band::High) => Rgb(0xe8, 0xa3, 0x3d),
        Some(Band::InRange) => Rgb(0x2e, 0x9d, 0x5b),
        None => Rgb(0x6b, 0x6f, 0x76),
    }
}

// 4x7 glyphs, one row per byte, high nibble is the leftmost column.
fn glyph(c: char) -> Option<(u32, [u8; 7])> {
    let rows = match c {
        '0' => [0x6, 0x9, 0xB, 0xD, 0x9, 0x9, 0x6],
        '1' => [0x2, 0x6, 0x2, 0x2, 0x2, 0x2, 0x7],
        '2' => [0x6, 0x9, 0x1, 0x2, 0x4, 0x8, 0xF],
        '3' => [0xE, 0x1, 0x1, 0x6, 0x1, 0x1, 0xE],
        '4' => [0x1, 0x3, 0x5, 0x9, 0xF, 0x1, 0x1],
        '5' => [0xF, 0x8, 0xE, 0x1, 0x1, 0x9, 0x6],
        '6' => [0x6, 0x8, 0x8, 0xE, 0x9, 0x9, 0x6],
        '7' => [0xF, 0x1, 0x2, 0x2, 0x4, 0x4, 0x4],
        '8' => [0x6, 0x9, 0x9, 0x6, 0x9, 0x9, 0x6],
        '9' => [0x6, 0x9, 0x9, 0x7, 0x1, 0x1, 0x6],
        '-' => [0x0, 0x0, 0x0, 0xF, 0x0, 0x0, 0x0],
        '.' => return Some((1, [0, 0, 0, 0, 0, 0, 0x8])),
        _ => return None,
    };
    Some((4, rows))
}

// 7x7 arrows, one row per byte, bit 6 is the leftmost column.
fn arrow(trend: Trend) -> Option<[u8; 7]> {
    Some(match trend {
        Trend::Flat | Trend::None => return None,
        Trend::SingleUp | Trend::DoubleUp => [0x08, 0x1C, 0x2A, 0x49, 0x08, 0x08, 0x08],
        Trend::SingleDown | Trend::DoubleDown => [0x08, 0x08, 0x08, 0x49, 0x2A, 0x1C, 0x08],
        Trend::FortyFiveUp => [0x0F, 0x03, 0x05, 0x09, 0x10, 0x20, 0x40],
        Trend::FortyFiveDown => [0x40, 0x20, 0x10, 0x09, 0x05, 0x03, 0x0F],
        Trend::NotComputable | Trend::RateOutOfRange => [0x1C, 0x22, 0x02, 0x04, 0x08, 0x00, 0x08],
    })
}

const FLAT: [u8; 7] = [0x00, 0x04, 0x02, 0x7F, 0x02, 0x04, 0x00];

struct Canvas {
    px: Vec<u8>,
}

impl Canvas {
    fn new() -> Canvas {
        Canvas { px: vec![0; (SIZE * SIZE * 4) as usize] }
    }

    fn set(&mut self, x: i32, y: i32, c: Rgb, a: u8) {
        if x < 0 || y < 0 || x >= SIZE as i32 || y >= SIZE as i32 {
            return;
        }
        let i = ((y as u32 * SIZE + x as u32) * 4) as usize;
        self.px[i..i + 4].copy_from_slice(&[c.0, c.1, c.2, a]);
    }

    fn rounded_rect(&mut self, c: Rgb, r: i32) {
        let s = SIZE as i32;
        for y in 0..s {
            for x in 0..s {
                let dx = if x < r { r - x } else if x >= s - r { x - (s - r - 1) } else { 0 };
                let dy = if y < r { r - y } else if y >= s - r { y - (s - r - 1) } else { 0 };
                if dx * dx + dy * dy <= r * r {
                    self.set(x, y, c, 255);
                }
            }
        }
    }

    fn circle(&mut self, c: Rgb, cx: f32, cy: f32, radius: f32) {
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let a = (radius + 0.5 - d).clamp(0.0, 1.0);
                if a > 0.0 {
                    self.set(x, y, c, (a * 255.0) as u8);
                }
            }
        }
    }

    fn bits(&mut self, x0: i32, y0: i32, rows: &[u8], width: u32, sx: i32, sy: i32, c: Rgb) {
        for (ry, row) in rows.iter().enumerate() {
            for col in 0..width {
                if row >> (width - 1 - col) & 1 == 1 {
                    for dy in 0..sy {
                        for dx in 0..sx {
                            self.set(x0 + col as i32 * sx + dx, y0 + ry as i32 * sy + dy, c, 255);
                        }
                    }
                }
            }
        }
    }
}

/// The Windows and Linux icon: the value in white on a band-coloured tile,
/// with the trend arrow underneath.
pub fn reading_icon(value: &str, trend: Trend, bg: Rgb) -> Vec<u8> {
    let mut cv = Canvas::new();
    cv.rounded_rect(bg, 6);
    let white = Rgb(255, 255, 255);
    let glyphs: Vec<(u32, [u8; 7])> = value.chars().filter_map(glyph).collect();
    let (sx, sy) = (2, 3);
    let gap = 1;
    let total: i32 = glyphs.iter().map(|(w, _)| *w as i32 * sx).sum::<i32>() + gap * sx * (glyphs.len() as i32 - 1).max(0);
    let sx = if total > SIZE as i32 - 2 { 1 } else { sx };
    let total: i32 = glyphs.iter().map(|(w, _)| *w as i32 * sx).sum::<i32>() + gap * sx * (glyphs.len() as i32 - 1).max(0);
    let mut x = (SIZE as i32 - total) / 2;
    for (w, rows) in &glyphs {
        cv.bits(x, 2, rows, *w, sx, sy, white);
        x += *w as i32 * sx + gap * sx;
    }
    let rows = match trend {
        Trend::Flat => Some(FLAT),
        other => arrow(other),
    };
    if let Some(rows) = rows {
        let double = matches!(trend, Trend::DoubleUp | Trend::DoubleDown);
        if double {
            cv.bits(8, 24, &rows, 7, 1, 1, white);
            cv.bits(17, 24, &rows, 7, 1, 1, white);
        } else {
            cv.bits(12, 24, &rows, 7, 1, 1, white);
        }
    }
    cv.px
}

/// The macOS icon: a coloured dot. The reading itself is the status item's
/// title, which macOS draws in the menu bar font beside it.
pub fn dot_icon(c: Rgb) -> Vec<u8> {
    let mut cv = Canvas::new();
    cv.circle(c, 16.0, 16.0, 9.0);
    cv.px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_have_the_right_size_and_some_ink() {
        let px = reading_icon("142", Trend::DoubleUp, band_color(Some(Band::InRange), false));
        assert_eq!(px.len(), (SIZE * SIZE * 4) as usize);
        let white = px.chunks(4).filter(|p| p == &[255, 255, 255, 255]).count();
        assert!(white > 60, "digits and arrows drawn: {white}");
        let mmol = reading_icon("10.5", Trend::Flat, band_color(Some(Band::High), false));
        assert_eq!(mmol.len(), (SIZE * SIZE * 4) as usize);
        let dot = dot_icon(band_color(None, true));
        assert!(dot.chunks(4).any(|p| p[3] == 255));
        assert!(dot.chunks(4).any(|p| p[3] == 0));
    }
}
