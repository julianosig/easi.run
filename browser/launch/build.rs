//! Rebuilds the crate when the `easi_files_v1` protocol changes: wayland-scanner reads its XML inside its macro
//! (src/files.rs) without telling Cargo, so a change to the XML alone would keep the old generated code.

fn main() {
    println!("cargo:rerun-if-changed=../../protocol/wayland/easi-files-v1.xml");
}
