//! Writes WS2812 colors to the simulated device, the same way `ws2812-spi` does.
//!
//! Usage: `cargo run --example spi_probe -- [device] [R,G,B]...`
//! Sends each color with `write(2)`, then one more through `SPI_IOC_MESSAGE` so both
//! paths of the simulator get exercised.

use spidev::{SpiModeFlags, Spidev, SpidevOptions, SpidevTransfer};
use std::io::Write;

/// Two WS2812 bits per SPI byte: `1110` is a one, `1000` a zero.
fn encode(r: u8, g: u8, b: u8) -> Vec<u8> {
    const PATTERNS: [u8; 4] = [0b1000_1000, 0b1000_1110, 0b1110_1000, 0b1110_1110];
    let mut out = Vec::new();

    for mut byte in [g, r, b] {
        for _ in 0..4 {
            out.push(PATTERNS[(byte >> 6) as usize]);
            byte <<= 2;
        }
    }

    out.extend(std::iter::repeat_n(0u8, 140)); // latch
    out
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "/dev/spidev0.0".to_string());
    let colors: Vec<String> = args.collect();
    let colors = if colors.is_empty() {
        vec![
            "255,0,0".into(),
            "0,255,0".into(),
            "255,255,0".into(),
            "255,255,255".into(),
            "0,0,0".into(),
        ]
    } else {
        colors
    };

    let mut spi = Spidev::open(&path)?;
    let options = SpidevOptions::new()
        .bits_per_word(8)
        .max_speed_hz(3_000_000)
        .mode(SpiModeFlags::SPI_MODE_0)
        .build();
    spi.configure(&options)?;

    for color in &colors {
        let parts: Vec<u8> = color.split(',').map(|p| p.trim().parse().unwrap_or(0)).collect();
        let (r, g, b) = (parts[0], *parts.get(1).unwrap_or(&0), *parts.get(2).unwrap_or(&0));

        spi.write_all(&encode(r, g, b))?;
        println!("wrote rgb({r}, {g}, {b}) with write()");
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    // A color of its own, so the ioctl path is visible in the simulator's output.
    let teal = encode(0, 128, 128);
    spi.transfer(&mut SpidevTransfer::write(&teal))?;
    println!("wrote rgb(0, 128, 128) with SPI_IOC_MESSAGE");

    Ok(())
}
