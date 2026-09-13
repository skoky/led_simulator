//! A WS2812 LED simulator: creates a virtual `/dev/spidev0.0` through CUSE, decodes the
//! WS2812 frames written to it, and shows the resulting colors in the terminal.
//!
//! Needs root (CUSE opens `/dev/cuse`); the created node is chmod'ed so the program under
//! test can run unprivileged.

mod cuse;
mod device;
mod display;
mod ws2812;

use crate::cuse::{CUSE_UNRESTRICTED_IOCTL, CuseInfo, cuse_lowlevel_main};
use clap::Parser;
use std::ffi::{CString, c_char, c_int};
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(version, about = "Virtual spidev device that prints the WS2812 colors written to it")]
struct Args {
    /// Device node to create (a path under /dev, or a bare name)
    #[arg(short, long, default_value = "/dev/spidev0.0", env = "LED_SIM_DEVICE")]
    device: String,

    /// Permissions for the created node, octal
    #[arg(short, long, default_value = "0666")]
    mode: String,

    /// Print every latched frame, not only the ones that change the color
    #[arg(short, long)]
    verbose: bool,

    /// Log opens, ioctls and byte counts to stderr
    #[arg(short, long)]
    trace: bool,

    /// Plain output, no ANSI colors
    #[arg(long)]
    no_color: bool,

    /// libfuse protocol debugging
    #[arg(long)]
    fuse_debug: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();

    // SAFETY: geteuid is always safe to call.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("led_simulator needs root to open /dev/cuse - re-run with sudo");
        return ExitCode::FAILURE;
    }

    let name = args.device.trim_start_matches("/dev/").to_string();
    let path = format!("/dev/{name}");

    if Path::new(&path).exists() {
        eprintln!("{path} already exists - remove it, or pick another name with --device");
        return ExitCode::FAILURE;
    }

    let mode = match u32::from_str_radix(args.mode.trim_start_matches("0o"), 8) {
        Ok(mode) => mode,
        Err(_) => {
            eprintln!("--mode must be octal, e.g. 0666");
            return ExitCode::FAILURE;
        }
    };

    // SAFETY: isatty only inspects the descriptor.
    let ansi = !args.no_color && unsafe { libc::isatty(libc::STDOUT_FILENO) } == 1;
    device::init(device::Config {
        verbose: args.verbose,
        trace: args.trace,
        ansi,
    });

    let watched = path.clone();
    std::thread::spawn(move || announce(&watched, mode));

    let rc = run_session(&name, args.fuse_debug);

    let (bytes, frames) = device::stats();
    println!("\nstopped: {bytes} bytes, {frames} frames");

    if rc == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// Hands control to libfuse. Returns when the session ends (Ctrl-C or device removal).
fn run_session(name: &str, fuse_debug: bool) -> c_int {
    let dev_info = CString::new(format!("DEVNAME={name}")).expect("device name without NUL");
    let dev_info_argv = [dev_info.as_ptr()];

    let info = CuseInfo {
        dev_major: 0, // let the kernel allocate
        dev_minor: 0,
        dev_info_argc: 1,
        dev_info_argv: dev_info_argv.as_ptr(),
        flags: CUSE_UNRESTRICTED_IOCTL,
    };

    // -f keeps us in the foreground, -s single-threaded so frames print in bus order.
    let mut argv = vec![c"led_simulator".to_owned(), c"-f".to_owned(), c"-s".to_owned()];
    if fuse_debug {
        argv.push(c"-d".to_owned());
    }
    let argv_ptrs: Vec<*mut c_char> = argv.iter().map(|a| a.as_ptr() as *mut c_char).collect();

    let ops = device::ops();

    // SAFETY: all pointers outlive the call, and libfuse only borrows them for its duration.
    unsafe {
        cuse_lowlevel_main(
            argv_ptrs.len() as c_int,
            argv_ptrs.as_ptr(),
            &info,
            &ops,
            std::ptr::null_mut(),
        )
    }
}

/// Waits for udev to create the node, relaxes its permissions and says we are ready.
fn announce(path: &str, mode: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);

    while !Path::new(path).exists() {
        if Instant::now() > deadline {
            eprintln!("{path} did not appear - is the cuse module available?");
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let c_path = CString::new(path).expect("path without NUL");
    // SAFETY: `c_path` is a valid NUL-terminated string for the duration of the call.
    if unsafe { libc::chmod(c_path.as_ptr(), mode as libc::mode_t) } != 0 {
        eprintln!("could not chmod {path}: {}", std::io::Error::last_os_error());
    }

    println!("{path} ready ({mode:04o}) - waiting for WS2812 frames, Ctrl-C to stop");
}
