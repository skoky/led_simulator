use std::path::Path;

/// Links libfuse3 without requiring `libfuse3-dev`: when the development symlink
/// (`libfuse3.so`) is missing we link the versioned soname directly.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let dirs = [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib64",
        "/usr/lib",
        "/usr/local/lib",
    ];

    for dir in dirs {
        if Path::new(&format!("{dir}/libfuse3.so")).exists() {
            println!("cargo:rustc-link-search=native={dir}");
            println!("cargo:rustc-link-lib=dylib=fuse3");
            return;
        }
    }

    for dir in dirs {
        for soname in ["libfuse3.so.4", "libfuse3.so.3"] {
            if Path::new(&format!("{dir}/{soname}")).exists() {
                println!("cargo:rustc-link-search=native={dir}");
                println!("cargo:rustc-link-arg=-l:{soname}");
                return;
            }
        }
    }

    panic!("libfuse3 not found - install it with: sudo apt install libfuse3-4 (or libfuse3-dev)");
}
