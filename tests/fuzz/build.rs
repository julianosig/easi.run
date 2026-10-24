//! Where `microphone_packet` finds libopus 1.5.2's static `libopus.a`, the one the display links: OPUS_LIB_DIR, as for
//! the display (scripts/opus.sh builds it; run.sh does when the variable is unset). No other target links it.

fn main() {
    println!("cargo:rerun-if-env-changed=OPUS_LIB_DIR");
    if let Ok(dir) = std::env::var("OPUS_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
    }
}
