//! Terminal rendering: colored blocks, hex values and a human name per pixel.

use crate::ws2812::Rgb;

/// Reference colors used to name a pixel. Compared after normalising brightness, so a
/// dimmed `rgb(0, 32, 0)` still reads as "green" rather than "off".
const NAMED: &[(&str, Rgb)] = &[
    ("red", Rgb { r: 255, g: 0, b: 0 }),
    ("green", Rgb { r: 0, g: 255, b: 0 }),
    ("blue", Rgb { r: 0, g: 0, b: 255 }),
    ("yellow", Rgb { r: 255, g: 255, b: 0 }),
    ("cyan", Rgb { r: 0, g: 255, b: 255 }),
    ("magenta", Rgb { r: 255, g: 0, b: 255 }),
    ("white", Rgb { r: 255, g: 255, b: 255 }),
    ("orange", Rgb { r: 255, g: 165, b: 0 }),
    ("purple", Rgb { r: 160, g: 32, b: 240 }),
];

/// `██` painted in the pixel's own color.
pub fn block(color: Rgb, ansi: bool) -> String {
    if !ansi {
        return "[]".to_string();
    }

    format!("\x1b[38;2;{};{};{}m██\x1b[0m", color.r, color.g, color.b)
}

pub fn hex(color: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
}

/// Nearest named color plus how bright it is, e.g. `green 13%`.
pub fn describe(color: Rgb) -> String {
    let max = color.r.max(color.g).max(color.b);
    if max == 0 {
        return "off".to_string();
    }

    // Normalise to full brightness so only the hue is matched.
    let scale = |v: u8| (v as u32 * 255 / max as u32) as i32;
    let (norm_r, norm_g, norm_b) = (scale(color.r), scale(color.g), scale(color.b));

    let name = NAMED
        .iter()
        .min_by_key(|(_, c)| (norm_r - c.r as i32).pow(2) + (norm_g - c.g as i32).pow(2) + (norm_b - c.b as i32).pow(2))
        .map(|(name, _)| *name)
        .unwrap_or("?");

    format!("{name} {}%", (max as u32 * 100).div_ceil(255))
}

/// `HH:MM:SS.mmm` in local time. Avoids a date/time dependency - `libc` is already here
/// for the CUSE bindings.
pub fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };

    // SAFETY: `secs` and `tm` are valid pointers to correctly sized values.
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return "??:??:??.???".to_string();
    }

    format!(
        "{:02}:{:02}:{:02}.{:03}",
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        now.subsec_millis()
    )
}

/// One log line for a latched frame.
pub fn frame_line(pixels: &[Rgb], ansi: bool) -> String {
    let ts = timestamp();

    match pixels {
        [] => format!("{ts}  (empty frame)"),
        [one] => format!(
            "{ts}  {}  {}  rgb({}, {}, {})  {}",
            block(*one, ansi),
            hex(*one),
            one.r,
            one.g,
            one.b,
            describe(*one)
        ),
        many => {
            let blocks: String = many.iter().map(|p| block(*p, ansi)).collect();
            let hexes: Vec<String> = many.iter().map(|p| hex(*p)).collect();
            format!("{ts}  {blocks}  {} LEDs  {}", many.len(), hexes.join(" "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_dimmed_colors_by_hue() {
        assert_eq!(describe(Rgb { r: 0, g: 32, b: 0 }), "green 13%");
        assert_eq!(describe(Rgb { r: 255, g: 255, b: 0 }), "yellow 100%");
        assert_eq!(describe(Rgb { r: 0, g: 0, b: 0 }), "off");
    }

    #[test]
    fn hex_is_uppercase_and_padded() {
        assert_eq!(hex(Rgb { r: 1, g: 255, b: 16 }), "#01FF10");
    }
}
