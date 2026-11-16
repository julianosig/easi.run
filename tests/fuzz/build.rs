//! Where `microphone_packet` finds libopus's static `libopus.a`, the one the display links: OPUS_LIB_DIR, as for
//! the display (scripts/opus.sh builds it; run.sh does when the variable is unset). No other target links it.

fn main() {
    println!("cargo:rerun-if-env-changed=OPUS_LIB_DIR");
    if let Ok(dir) = std::env::var("OPUS_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
        // Linked statically, from outside the package: built anew at the same path (run.sh builds it when the
        // variable is unset), it must be linked anew, which Cargo does only for what it is told to watch.
        let archive = std::path::Path::new(&dir).join("libopus.a");
        if archive.exists() {
            println!("cargo:rerun-if-changed={}", archive.display());
        }
    }
}
