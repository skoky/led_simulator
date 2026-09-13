//! The virtual spidev: CUSE callbacks, the SPI ioctl surface, and the shared state that
//! turns incoming bytes into printed colors.

use crate::cuse::*;
use crate::display;
use crate::ws2812::{Decoder, Rgb};
use std::ffi::{c_char, c_int, c_uint, c_void};
use std::sync::{Mutex, OnceLock};

/// `SPI_IOC_MAGIC` from `include/uapi/linux/spi/spidev.h`.
const SPI_IOC_MAGIC: u32 = b'k' as u32;
/// Refuse absurd transfer counts rather than building a huge iovec (the kernel caps the
/// vector at `FUSE_IOCTL_MAX_IOV` = 256 entries anyway).
const MAX_TRANSFERS: usize = 64;

/// `struct spi_ioc_transfer` - 32 bytes, must match the kernel layout exactly.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct SpiIocTransfer {
    tx_buf: u64,
    rx_buf: u64,
    len: u32,
    speed_hz: u32,
    delay_usecs: u16,
    bits_per_word: u8,
    cs_change: u8,
    tx_nbits: u8,
    rx_nbits: u8,
    word_delay_usecs: u8,
    pad: u8,
}

const TRANSFER_SIZE: usize = size_of::<SpiIocTransfer>();

/// Bus settings the client configured, remembered so read-back ioctls answer sensibly.
#[derive(Default)]
struct SpiConfig {
    mode: u8,
    lsb_first: u8,
    bits_per_word: u8,
    max_speed_hz: u32,
}

pub struct Config {
    /// Print every latched frame, not only the ones that change the color.
    pub verbose: bool,
    /// Log ioctls, opens and byte counts.
    pub trace: bool,
    pub ansi: bool,
}

pub struct Sim {
    decoder: Decoder,
    config: Config,
    spi: SpiConfig,
    last: Option<Vec<Rgb>>,
    bytes: u64,
    frames: u64,
}

static SIM: OnceLock<Mutex<Sim>> = OnceLock::new();

pub fn init(config: Config) {
    let sim = Sim {
        decoder: Decoder::new(),
        config,
        spi: SpiConfig::default(),
        last: None,
        bytes: 0,
        frames: 0,
    };
    let _ = SIM.set(Mutex::new(sim));
}

fn sim() -> std::sync::MutexGuard<'static, Sim> {
    SIM.get()
        .expect("state initialised before the session starts")
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

impl Sim {
    /// Feeds raw SPI bytes to the decoder and prints whatever latched.
    fn consume(&mut self, data: &[u8]) {
        self.bytes += data.len() as u64;

        for frame in self.decoder.feed(data) {
            self.frames += 1;

            if frame.trailing_bits != 0 && self.config.trace {
                eprintln!(
                    "{}  incomplete pixel: {} stray bits",
                    display::timestamp(),
                    frame.trailing_bits
                );
            }

            // An empty frame is just the idle line - never interesting on its own.
            let changed = self.last.as_deref() != Some(frame.pixels.as_slice());
            if frame.pixels.is_empty() || !(changed || self.config.verbose) {
                continue;
            }

            println!("{}", display::frame_line(&frame.pixels, self.config.ansi));
            self.last = Some(frame.pixels);
        }
    }

    fn trace(&self, msg: impl AsRef<str>) {
        if self.config.trace {
            eprintln!("{}  {}", display::timestamp(), msg.as_ref());
        }
    }
}

/// Summary printed on shutdown.
pub fn stats() -> (u64, u64) {
    let sim = sim();
    (sim.bytes, sim.frames)
}

// --- CUSE callbacks ---------------------------------------------------------------

extern "C" fn open_cb(req: FuseReq, fi: *mut c_void) {
    sim().trace("open");
    // SAFETY: `fi` is the request's own file info, passed straight back to libfuse.
    unsafe { fuse_reply_open(req, fi) };
}

extern "C" fn release_cb(req: FuseReq, _fi: *mut c_void) {
    sim().trace("release");
    unsafe { fuse_reply_err(req, 0) };
}

extern "C" fn flush_cb(req: FuseReq, _fi: *mut c_void) {
    unsafe { fuse_reply_err(req, 0) };
}

extern "C" fn fsync_cb(req: FuseReq, _datasync: c_int, _fi: *mut c_void) {
    unsafe { fuse_reply_err(req, 0) };
}

/// Reading a WS2812 line returns nothing useful; real spidev hands back zeros.
extern "C" fn read_cb(req: FuseReq, size: usize, _off: i64, _fi: *mut c_void) {
    let zeros = vec![0u8; size.min(64 * 1024)];
    unsafe { fuse_reply_buf(req, zeros.as_ptr() as *const c_char, zeros.len()) };
}

extern "C" fn write_cb(req: FuseReq, buf: *const c_char, size: usize, _off: i64, _fi: *mut c_void) {
    // SAFETY: libfuse guarantees `buf` points to `size` readable bytes for this call.
    let data = unsafe { std::slice::from_raw_parts(buf as *const u8, size) };

    let mut sim = sim();
    sim.trace(format!("write {size} B"));
    sim.consume(data);
    drop(sim);

    unsafe { fuse_reply_write(req, size) };
}

extern "C" fn ioctl_cb(
    req: FuseReq,
    cmd: c_int,
    arg: *mut c_void,
    _fi: *mut c_void,
    flags: c_uint,
    in_buf: *const c_void,
    in_bufsz: usize,
    out_bufsz: usize,
) {
    if flags & FUSE_IOCTL_COMPAT != 0 {
        unsafe { fuse_reply_err(req, libc::ENOSYS) };
        return;
    }

    let cmd = cmd as u32;
    if ioc::type_(cmd) != SPI_IOC_MAGIC {
        sim().trace(format!("unknown ioctl {cmd:#010x}"));
        unsafe { fuse_reply_err(req, libc::ENOTTY) };
        return;
    }

    // SPI_IOC_MESSAGE(N) is _IOW('k', 0, ...) with the transfer count folded into the size.
    if ioc::nr(cmd) == 0 && ioc::dir(cmd) == ioc::WRITE {
        spi_message(req, cmd, arg, in_buf, in_bufsz, out_bufsz);
        return;
    }

    config_ioctl(req, cmd, arg, in_buf, in_bufsz, out_bufsz);
}

/// Mode / bits-per-word / speed, in both directions.
fn config_ioctl(req: FuseReq, cmd: u32, arg: *mut c_void, in_buf: *const c_void, in_bufsz: usize, out_bufsz: usize) {
    let size = ioc::size(cmd) as usize;
    let dir = ioc::dir(cmd);
    let wants_in = dir & ioc::WRITE != 0;
    let wants_out = dir & ioc::READ != 0;

    // CUSE ioctls are always "unrestricted": nothing has been copied yet, so ask the
    // kernel to fetch the argument and deliver the same ioctl again.
    if (wants_in && in_bufsz < size) || (wants_out && out_bufsz < size) {
        let iov = [libc::iovec {
            iov_base: arg,
            iov_len: size,
        }];
        let empty: [libc::iovec; 0] = [];
        let in_iov: &[libc::iovec] = if wants_in { &iov } else { &empty };
        let out_iov: &[libc::iovec] = if wants_out { &iov } else { &empty };

        retry(req, in_iov, out_iov);
        return;
    }

    let mut sim = sim();

    if wants_in {
        // SAFETY: the kernel copied `in_bufsz` >= `size` bytes for us.
        let value = unsafe { std::slice::from_raw_parts(in_buf as *const u8, size) };
        let word = |n: usize| value.iter().take(n).rev().fold(0u32, |a, b| a << 8 | *b as u32);

        match ioc::nr(cmd) {
            1 => sim.spi.mode = value[0],
            2 => sim.spi.lsb_first = value[0],
            3 => sim.spi.bits_per_word = value[0],
            4 => sim.spi.max_speed_hz = word(4),
            5 => sim.spi.mode = word(4) as u8,
            _ => {}
        }

        sim.trace(format!(
            "configure: mode {:#04x}, {} bits/word, {} Hz, lsb_first {}",
            sim.spi.mode, sim.spi.bits_per_word, sim.spi.max_speed_hz, sim.spi.lsb_first
        ));
    }

    if wants_out {
        let value: u32 = match ioc::nr(cmd) {
            1 => sim.spi.mode as u32,
            2 => sim.spi.lsb_first as u32,
            3 => sim.spi.bits_per_word as u32,
            4 => sim.spi.max_speed_hz,
            5 => sim.spi.mode as u32,
            _ => 0,
        };
        let bytes = value.to_le_bytes();
        drop(sim);

        unsafe { fuse_reply_ioctl(req, 0, bytes.as_ptr() as *const c_void, size.min(bytes.len()) as u32) };
        return;
    }

    drop(sim);
    unsafe { fuse_reply_ioctl(req, 0, std::ptr::null(), 0) };
}

/// `SPI_IOC_MESSAGE(N)`: an array of transfers whose `tx_buf`/`rx_buf` are pointers into
/// the caller's address space, so the payload takes a second retry round to collect.
fn spi_message(req: FuseReq, cmd: u32, arg: *mut c_void, in_buf: *const c_void, in_bufsz: usize, out_bufsz: usize) {
    let array_size = ioc::size(cmd) as usize;
    let count = array_size / TRANSFER_SIZE;

    if count == 0 || count > MAX_TRANSFERS {
        unsafe { fuse_reply_err(req, libc::EINVAL) };
        return;
    }

    // Round one: fetch the transfer array itself.
    if in_bufsz < array_size {
        let iov = libc::iovec {
            iov_base: arg,
            iov_len: array_size,
        };
        retry(req, &[iov], &[]);
        return;
    }

    // SAFETY: the kernel copied at least `array_size` bytes, and `SpiIocTransfer` mirrors
    // the kernel struct, so the prefix of the buffer is a valid array of `count` of them.
    let transfers = unsafe { std::slice::from_raw_parts(in_buf as *const SpiIocTransfer, count) }.to_vec();

    let tx_total: usize = transfers.iter().filter(|t| t.tx_buf != 0).map(|t| t.len as usize).sum();
    let rx_total: usize = transfers.iter().filter(|t| t.rx_buf != 0).map(|t| t.len as usize).sum();

    // Round two: fetch the tx payloads (and reserve room for the rx ones).
    if in_bufsz < array_size + tx_total || out_bufsz < rx_total {
        let mut in_iov = vec![libc::iovec {
            iov_base: arg,
            iov_len: array_size,
        }];
        let mut out_iov = Vec::new();

        for t in &transfers {
            if t.tx_buf != 0 && t.len > 0 {
                in_iov.push(libc::iovec {
                    iov_base: t.tx_buf as *mut c_void,
                    iov_len: t.len as usize,
                });
            }
            if t.rx_buf != 0 && t.len > 0 {
                out_iov.push(libc::iovec {
                    iov_base: t.rx_buf as *mut c_void,
                    iov_len: t.len as usize,
                });
            }
        }

        retry(req, &in_iov, &out_iov);
        return;
    }

    // SAFETY: as above, the kernel delivered `array_size + tx_total` bytes.
    let payload = unsafe { std::slice::from_raw_parts(in_buf as *const u8, array_size + tx_total) };
    let mut offset = array_size;
    let mut sim = sim();
    sim.trace(format!(
        "SPI_IOC_MESSAGE: {count} transfer(s), {tx_total} B out, {rx_total} B in"
    ));

    for t in &transfers {
        if t.tx_buf != 0 && t.len > 0 {
            let end = offset + t.len as usize;
            sim.consume(&payload[offset..end]);
            offset = end;
        }
    }
    drop(sim);

    // A real controller would return what the device clocked back; nothing is driving
    // MISO here, so the read side is zeros.
    let total: usize = transfers.iter().map(|t| t.len as usize).sum();
    let zeros = vec![0u8; rx_total];
    unsafe { fuse_reply_ioctl(req, total as c_int, zeros.as_ptr() as *const c_void, rx_total as u32) };
}

fn retry(req: FuseReq, in_iov: &[libc::iovec], out_iov: &[libc::iovec]) {
    let ptr = |iov: &[libc::iovec]| if iov.is_empty() { std::ptr::null() } else { iov.as_ptr() };

    unsafe { fuse_reply_ioctl_retry(req, ptr(in_iov), in_iov.len(), ptr(out_iov), out_iov.len()) };
}

pub fn ops() -> CuseLowlevelOps {
    CuseLowlevelOps {
        open: Some(open_cb),
        read: Some(read_cb),
        write: Some(write_cb),
        flush: Some(flush_cb),
        release: Some(release_cb),
        fsync: Some(fsync_cb),
        ioctl: Some(ioctl_cb),
        ..Default::default()
    }
}
