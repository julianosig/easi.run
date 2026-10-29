//! Names the target triple the tests are built for, which Cargo puts in their target directory's path when the build
//! names one (src/compositor.rs builds the display binary beside them).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=TARGET={}", std::env::var("TARGET").expect("cargo sets TARGET"));
}
