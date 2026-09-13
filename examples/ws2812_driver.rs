//! Drives the simulated device with the real driver stack - `ws2812-spi` on top of
//! `linux-embedded-hal`'s `SpidevBus` - exactly as `pnp`'s `led/raspi.rs` does.
//!
//! Usage: `cargo run --example ws2812_driver -- [device] [brightness]`

use linux_embedded_hal::SpidevBus;
use smart_leds::{RGB8, SmartLedsWrite, brightness};
use spidev::{SpiModeFlags, Spidev, SpidevOptions};
use ws2812_spi::Ws2812;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "/dev/spidev0.0".to_string());
    let level: u8 = args.next().and_then(|a| a.parse().ok()).unwrap_or(255);

    let mut spi = Spidev::open(&path)?;
    let options = SpidevOptions::new()
        .bits_per_word(8)
        .max_speed_hz(3_000_000)
        .mode(SpiModeFlags::SPI_MODE_0)
        .build();
    spi.configure(&options)?;

    let mut ws = Ws2812::new(SpidevBus(spi));

    // The colors pnp uses: red, green, yellow, white, off.
    let sequence = [
        ("red", RGB8::new(255, 0, 0)),
        ("green", RGB8::new(0, 255, 0)),
        ("yellow", RGB8::new(255, 255, 0)),
        ("white", RGB8::new(255, 255, 255)),
        ("off", RGB8::new(0, 0, 0)),
    ];

    for (name, color) in sequence {
        println!("sending {name} at brightness {level}");
        ws.write(brightness([color].into_iter(), level))
            .expect("write to the simulated bus");
        std::thread::sleep(std::time::Duration::from_millis(400));
    }

    Ok(())
}
