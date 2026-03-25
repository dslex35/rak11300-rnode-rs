//! Build script for rak11300-rnode
//!
//! cortex-m-rt's link.x contains:
//!     INCLUDE memory.x
//!
//! The linker only searches the directories listed via -L flags.  The project
//! root is not added automatically, so without this script the linker cannot
//! find memory.x and the build fails with:
//!     cannot find linker script memory.x
//!
//! This script adds CARGO_MANIFEST_DIR (the project root) to the linker
//! search path, making memory.x visible to rust-lld.

fn main() {
    // Add the project root to the linker search path.
    println!(
        "cargo:rustc-link-search={}",
        std::env::var("CARGO_MANIFEST_DIR").unwrap()
    );

    // Re-run this script if either file changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=memory.x");
}
