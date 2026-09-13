# led_simulator

A virtual `/dev/spidev0.0` that decodes the WS2812 frames written to it, prints the
resulting colors in the terminal and mirrors them in a small always-on-top window. It
exists so a WS2812 driver can be developed and tested on an ordinary Linux box, with no
Raspberry Pi and no LED strip attached.

```
18:53:04.618  ██  #FF0000  rgb(255, 0, 0)  red 100%
18:53:05.024  ██  #00FF00  rgb(0, 255, 0)  green 100%
18:53:07.469  ██  #202000  rgb(32, 32, 0)  yellow 13%
```

## How it works

The device node is a real character device created through **CUSE** (character device in
userspace, part of FUSE) — so `open`, the `SPI_IOC_*` ioctls, `write(2)` and
`SPI_IOC_MESSAGE` all behave like the real thing and no kernel module is needed.

Incoming bytes are the WS2812 waveform as a driver clocked it out over MOSI. Each WS2812
bit is a pulse: a wide high phase is a `1`, a narrow one a `0`. The decoder measures the
pulses rather than assuming one encoding, so both the 4-bits-per-bit scheme `ws2812-spi`
uses at 3 MHz (`1110` / `1000`) and the 3-bit scheme other drivers use at 2.4 MHz work.
A long low phase is the WS2812 latch and ends a frame; the bytes of a frame are read as
`G, R, B` triplets.

## Requirements

- `libfuse3` (`libfuse3-4` or `libfuse3-3`; the `-dev` package is *not* needed)
- the `cuse` kernel module, i.e. `/dev/cuse` — standard on Ubuntu
- root, because CUSE needs `/dev/cuse`

## Usage

```bash
cargo build --release
sudo ./target/release/led_simulator
```

The node is created with mode `0666`, so the program under test runs unprivileged.
`Ctrl-C` removes it again.

| Flag | Meaning |
| --- | --- |
| `-d, --device <PATH>` | node to create, default `/dev/spidev0.0` (env `LED_SIM_DEVICE`) |
| `-m, --mode <OCTAL>` | permissions for the node, default `0666` |
| `-v, --verbose` | print every frame, not only the ones that change the color |
| `-t, --trace` | log opens, ioctls and byte counts to stderr |
| `--no-color` | plain output (also automatic when stdout is not a terminal) |
| `--no-popup` | do not open the color window, terminal output only |
| `--fuse-debug` | libfuse protocol debugging |

Only color *changes* are printed by default: a driver refreshing the same color 50×/s
would otherwise bury the interesting transitions.

## The popup

A small window shows the current color as a swatch (one per LED for a strip) with its hex
value and name, and asks the window manager to keep it above other windows. Close it and
the simulator keeps running; `--no-popup` skips it entirely.

Always-on-top is an X11 request — Wayland has no protocol for it — so the window is opened
through X11, which on a Wayland desktop means XWayland. CUSE forces the process to run as
root, and root inherits neither the desktop's X11 cookie nor its Wayland socket, so the
`XAUTHORITY` of the user who ran `sudo` is located automatically (GNOME's
`.mutter-Xwaylandauth*`, `gdm/Xauthority` or `~/.Xauthority`). If the window still cannot
open, the failure is reported and the simulator carries on serving the device; `sudo -E`
passes your whole environment through and usually fixes it.

## Trying it out

With the simulator running, in another shell:

```bash
cargo run --example ws2812_driver -- /dev/spidev0.0 32
```

`ws2812_driver` drives the device through the real `ws2812-spi` + `linux-embedded-hal`
stack (the second argument is brightness, 0–255). `spi_probe` is the lower-level variant:
it writes hand-encoded frames with `write(2)` and one through `SPI_IOC_MESSAGE`.

```bash
cargo run --example spi_probe -- /dev/spidev0.0 255,0,0 0,255,0
```

## Using it with pnp

`pnp` opens `/dev/spidev0.0` on every LED change ([`crates/pnp/src/led/raspi.rs`]), so with
the simulator running its LED output shows up as colored lines and in the popup — red on start, green when
all is well, yellow while the bed moves, white while the controls are blocked.

Note that `ws2812-spi` writes **one SPI byte per syscall** (152 syscalls per frame), which
`--trace` makes very visible. That is the driver's behaviour, not the simulator's.

## Tests

```bash
cargo test
```

The decoder tests encode frames the way `ws2812-spi` does and assert the round trip,
including dimmed colors, frames split across arbitrary write boundaries, and the last bit
of a frame running into the latch.
