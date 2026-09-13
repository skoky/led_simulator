//! Turns a raw WS2812-over-SPI byte stream back into pixels.
//!
//! Drivers encode every WS2812 bit as a short pulse inside a few SPI bits: a wide high
//! phase is a `1`, a narrow one a `0`. `ws2812-spi` (what `pnp` uses) picks 4 SPI bits per
//! WS2812 bit at 3 MHz — `1110` / `1000` — while other drivers use 3 bits (`110` / `100`).
//! Instead of hard-coding one of them the decoder measures each pulse and compares the
//! high phase against the low phase, which covers every ratio-based encoding.
//!
//! A low phase longer than [`RESET_BITS`] is the latch (WS2812 reset), and ends a frame.
//! The final bit of a frame runs straight into that latch, so its low phase never ends;
//! it is decided against the symbol period learned from the preceding bits instead.

/// Low bits in a row that count as a reset/latch. `ws2812-spi` sends 140 zero bytes
/// (1120 bits); a single encoded bit never holds more than ~3 low bits in a row.
pub const RESET_BITS: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// One latched WS2812 update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub pixels: Vec<Rgb>,
    /// Bits that arrived after the last whole pixel — a sign the stream was cut short.
    pub trailing_bits: u32,
}

#[derive(Default)]
pub struct Decoder {
    high_len: u32,
    low_len: u32,
    /// SPI bits per WS2812 bit, learned from the stream (4 for `ws2812-spi`, 3 for others).
    period: u32,
    /// Decoded payload bits, packed MSB first.
    acc: u8,
    acc_bits: u32,
    bytes: Vec<u8>,
    latched: bool,
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            latched: true,
            ..Default::default()
        }
    }

    /// Feeds raw SPI bytes, returning every frame that latched within them.
    pub fn feed(&mut self, data: &[u8]) -> Vec<Frame> {
        let mut frames = Vec::new();

        for byte in data {
            for shift in (0..8).rev() {
                if let Some(frame) = self.push_level(byte >> shift & 1 == 1) {
                    frames.push(frame);
                }
            }
        }

        frames
    }

    /// Feeds one bit of the waveform. A pulse is decided once its low phase ends: a high
    /// phase wider than the low one is a `1`. The last pulse before the latch has no low
    /// phase to compare against, so it is measured against the learned symbol period.
    fn push_level(&mut self, high: bool) -> Option<Frame> {
        if high {
            // The low run ends here: close the pulse, or drop a run left over from a latch.
            if self.low_len > 0 {
                if self.high_len > 0 {
                    self.period = self.high_len + self.low_len;
                    let bit = self.high_len > self.low_len;
                    self.push_bit(bit);
                }

                self.high_len = 0;
                self.low_len = 0;
            }

            self.high_len += 1;
            self.latched = false;
            return None;
        }

        self.low_len += 1;
        if self.low_len < RESET_BITS {
            return None;
        }

        if self.high_len > 0 {
            let bit = match self.period {
                0 => self.high_len >= 2, // nothing learned yet: a wide pulse is a one
                period => self.high_len * 2 > period,
            };
            self.push_bit(bit);
            self.high_len = 0;
        }

        (!self.latched).then(|| self.latch())
    }

    fn push_bit(&mut self, bit: bool) {
        self.acc = self.acc << 1 | u8::from(bit);
        self.acc_bits += 1;

        if self.acc_bits == 8 {
            self.bytes.push(self.acc);
            self.acc = 0;
            self.acc_bits = 0;
        }
    }

    /// Ends the frame: WS2812 pixels arrive as G, R, B triplets.
    fn latch(&mut self) -> Frame {
        let pixels = self
            .bytes
            .chunks_exact(3)
            .map(|grb| Rgb {
                r: grb[1],
                g: grb[0],
                b: grb[2],
            })
            .collect();
        let trailing_bits = (self.bytes.len() % 3) as u32 * 8 + self.acc_bits;

        self.bytes.clear();
        self.acc = 0;
        self.acc_bits = 0;
        self.latched = true;

        Frame { pixels, trailing_bits }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The encoding `ws2812-spi` writes at 3 MHz: two WS2812 bits per SPI byte.
    fn encode_4bit(pixels: &[Rgb]) -> Vec<u8> {
        const PATTERNS: [u8; 4] = [0b1000_1000, 0b1000_1110, 0b1110_1000, 0b1110_1110];
        let mut out = Vec::new();

        for px in pixels {
            for mut byte in [px.g, px.r, px.b] {
                for _ in 0..4 {
                    out.push(PATTERNS[(byte >> 6) as usize]);
                    byte <<= 2;
                }
            }
        }

        out.extend(std::iter::repeat_n(0u8, 140)); // reset
        out
    }

    /// The 3-bits-per-bit encoding other drivers use at 2.4 MHz.
    fn encode_3bit(pixels: &[Rgb]) -> Vec<u8> {
        let mut bits = Vec::new();

        for px in pixels {
            for byte in [px.g, px.r, px.b] {
                for shift in (0..8).rev() {
                    bits.extend_from_slice(if byte >> shift & 1 == 1 { &[1, 1, 0] } else { &[1, 0, 0] });
                }
            }
        }

        let mut out: Vec<u8> = bits.chunks(8).map(|c| c.iter().fold(0u8, |a, b| a << 1 | b)).collect();
        out.extend(std::iter::repeat_n(0u8, 140));
        out
    }

    #[test]
    fn decodes_single_pixel() {
        let green = Rgb { r: 0, g: 255, b: 0 };
        let frames = Decoder::new().feed(&encode_4bit(&[green]));

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].pixels, vec![green]);
        assert_eq!(frames[0].trailing_bits, 0);
    }

    #[test]
    fn decodes_dimmed_and_mixed_colors() {
        let pixels = [
            Rgb { r: 0, g: 32, b: 0 },
            Rgb { r: 255, g: 255, b: 0 },
            Rgb { r: 0, g: 0, b: 0 },
            Rgb { r: 1, g: 2, b: 3 },
        ];
        let frames = Decoder::new().feed(&encode_4bit(&pixels));

        assert_eq!(frames[0].pixels, pixels);
    }

    #[test]
    fn decodes_three_bit_encoding() {
        let pixels = [Rgb { r: 255, g: 165, b: 0 }];
        let frames = Decoder::new().feed(&encode_3bit(&pixels));

        assert_eq!(frames[0].pixels, pixels.to_vec());
    }

    /// `ws2812-spi` writes one SPI byte per syscall, so the decoder must not depend on
    /// where the stream is chopped up.
    #[test]
    fn survives_arbitrary_write_boundaries() {
        let pixels = [Rgb { r: 10, g: 20, b: 30 }, Rgb { r: 255, g: 0, b: 128 }];
        let data = encode_4bit(&pixels);
        let mut decoder = Decoder::new();
        let mut frames = Vec::new();

        for byte in &data {
            frames.extend(decoder.feed(&[*byte]));
        }

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].pixels, pixels.to_vec());
    }

    #[test]
    fn back_to_back_frames_latch_separately() {
        let red = Rgb { r: 255, g: 0, b: 0 };
        let blue = Rgb { r: 0, g: 0, b: 255 };
        let mut data = encode_4bit(&[red]);
        data.extend(encode_4bit(&[blue]));
        let frames = Decoder::new().feed(&data);

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].pixels, vec![red]);
        assert_eq!(frames[1].pixels, vec![blue]);
    }

    /// Idle zeros on the bus are not a frame - the line simply rests low.
    #[test]
    fn idle_line_produces_no_frames() {
        assert!(Decoder::new().feed(&[0u8; 512]).is_empty());
    }

    /// A frame whose last bit is a one: that pulse runs straight into the latch.
    #[test]
    fn decodes_trailing_one_bit() {
        let blue = Rgb { r: 0, g: 0, b: 255 };
        let frames = Decoder::new().feed(&encode_4bit(&[blue]));

        assert_eq!(frames[0].pixels, vec![blue]);
    }
}
