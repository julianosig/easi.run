//! Compiles the SVT-AV1 shim and links SVT-AV1 and libopus statically. SVTAV1_INCLUDE_DIR and SVTAV1_LIB_DIR name
//! SVT-AV1 3.1.2's installed headers and `libSvtAv1Enc.a`, and OPUS_LIB_DIR libopus 1.5.2's `libopus.a` (the display
//! image builds both from source; see display/Dockerfile).

fn main() {
    println!("cargo:rerun-if-changed=src/stream/av1.c");
    // wayland-scanner reads this inside its macro (src/files.rs) without telling Cargo, so a change to it alone would
    // keep the old generated code; rerunning this script rebuilds the crate.
    println!("cargo:rerun-if-changed=../protocol/wayland/easi-files-v1.xml");
    println!("cargo:rerun-if-env-changed=SVTAV1_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=SVTAV1_LIB_DIR");
    println!("cargo:rerun-if-env-changed=OPUS_LIB_DIR");
    let mut build = cc::Build::new();
    build.file("src/stream/av1.c").warnings(true).extra_warnings(true).cargo_metadata(false);
    if let Ok(dir) = std::env::var("SVTAV1_INCLUDE_DIR") {
        build.include(dir);
    }
    build.compile("display_av1");
    // Static archives resolve left to right: the shim, then the library it calls.
    let out = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    println!("cargo:rustc-link-search=native={out}");
    if let Ok(dir) = std::env::var("SVTAV1_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
    }
    println!("cargo:rustc-link-lib=static=display_av1");
    println!("cargo:rustc-link-lib=static=SvtAv1Enc");
    if let Ok(dir) = std::env::var("OPUS_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
    }
    println!("cargo:rustc-link-lib=static=opus");
}
