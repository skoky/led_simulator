//! Hand-written bindings to libfuse3's CUSE ("character device in userspace") API.
//!
//! Only the handful of entry points this simulator needs are declared, which keeps
//! `libfuse3-dev` (and bindgen) out of the build - the shared library alone is enough.
//! See `cuse_lowlevel.h` and `fuse_lowlevel.h` in the libfuse sources for the originals.

use std::ffi::{c_char, c_int, c_uint, c_void};

/// Opaque `fuse_req_t`: the request being answered.
pub type FuseReq = *mut c_void;

/// Kernel may send any ioctl through, sizes are ours to work out.
pub const CUSE_UNRESTRICTED_IOCTL: c_uint = 1 << 0;

pub const FUSE_IOCTL_COMPAT: c_uint = 1 << 0;

#[repr(C)]
pub struct CuseInfo {
    pub dev_major: c_uint,
    pub dev_minor: c_uint,
    pub dev_info_argc: c_uint,
    pub dev_info_argv: *const *const c_char,
    pub flags: c_uint,
}

/// `struct cuse_lowlevel_ops`. Field order is ABI - do not rearrange.
#[repr(C)]
#[derive(Default)]
pub struct CuseLowlevelOps {
    pub init: Option<extern "C" fn(userdata: *mut c_void, conn: *mut c_void)>,
    pub init_done: Option<extern "C" fn(userdata: *mut c_void)>,
    pub destroy: Option<extern "C" fn(userdata: *mut c_void)>,
    pub open: Option<extern "C" fn(req: FuseReq, fi: *mut c_void)>,
    pub read: Option<extern "C" fn(req: FuseReq, size: usize, off: i64, fi: *mut c_void)>,
    pub write: Option<extern "C" fn(req: FuseReq, buf: *const c_char, size: usize, off: i64, fi: *mut c_void)>,
    pub flush: Option<extern "C" fn(req: FuseReq, fi: *mut c_void)>,
    pub release: Option<extern "C" fn(req: FuseReq, fi: *mut c_void)>,
    pub fsync: Option<extern "C" fn(req: FuseReq, datasync: c_int, fi: *mut c_void)>,
    pub ioctl: Option<
        extern "C" fn(
            req: FuseReq,
            cmd: c_int,
            arg: *mut c_void,
            fi: *mut c_void,
            flags: c_uint,
            in_buf: *const c_void,
            in_bufsz: usize,
            out_bufsz: usize,
        ),
    >,
    pub poll: Option<extern "C" fn(req: FuseReq, fi: *mut c_void, ph: *mut c_void)>,
}

unsafe extern "C" {
    pub fn cuse_lowlevel_main(
        argc: c_int,
        argv: *const *mut c_char,
        ci: *const CuseInfo,
        clop: *const CuseLowlevelOps,
        userdata: *mut c_void,
    ) -> c_int;

    pub fn fuse_reply_err(req: FuseReq, err: c_int) -> c_int;
    pub fn fuse_reply_open(req: FuseReq, fi: *const c_void) -> c_int;
    pub fn fuse_reply_write(req: FuseReq, count: usize) -> c_int;
    pub fn fuse_reply_buf(req: FuseReq, buf: *const c_char, size: usize) -> c_int;
    pub fn fuse_reply_ioctl(req: FuseReq, result: c_int, buf: *const c_void, size: u32) -> c_int;
    pub fn fuse_reply_ioctl_retry(
        req: FuseReq,
        in_iov: *const libc::iovec,
        in_count: usize,
        out_iov: *const libc::iovec,
        out_count: usize,
    ) -> c_int;
}

/// Fields packed into an ioctl request number (`include/uapi/asm-generic/ioctl.h`).
pub mod ioc {
    pub const WRITE: u32 = 1;
    pub const READ: u32 = 2;

    pub const fn dir(cmd: u32) -> u32 {
        cmd >> 30 & 0x3
    }

    pub const fn type_(cmd: u32) -> u32 {
        cmd >> 8 & 0xff
    }

    pub const fn nr(cmd: u32) -> u32 {
        cmd & 0xff
    }

    pub const fn size(cmd: u32) -> u32 {
        cmd >> 16 & 0x3fff
    }
}

#[cfg(test)]
mod tests {
    use super::ioc;

    /// `SPI_IOC_WR_MAX_SPEED_HZ` = `_IOW('k', 4, __u32)` = 0x40046b04.
    #[test]
    fn decodes_spi_ioctl_numbers() {
        let cmd: u32 = 0x4004_6b04;

        assert_eq!(ioc::dir(cmd), ioc::WRITE);
        assert_eq!(ioc::type_(cmd), b'k' as u32);
        assert_eq!(ioc::nr(cmd), 4);
        assert_eq!(ioc::size(cmd), 4);
    }
}
