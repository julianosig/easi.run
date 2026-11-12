# Fallback license texts

The license files of crates whose published package ships none, taken from their repositories, which
`scripts/notices.py` adds for the version each directory names (`<crate>-<version>`): a new version
has no fallback until its files are checked and copied here again.

- `webauthn-attestation-ca`, `webauthn-rs`, `webauthn-rs-core`, `webauthn-rs-proto` 0.6.2: `LICENSE.md` of
  https://github.com/kanidm/webauthn-rs at ed367d0316a946c86dd402404a9abbafbcc2165d.
- `crypto-glue` 0.2.2 (https://github.com/kanidm/crypto-glue at e40f0c6ecd5557550fec8ebbd2861521d92ee1a5): its
  repository holds no license file and its `Cargo.toml` declares MPL-2.0, so the file here is the Mozilla Public
  License 2.0 as the same project's webauthn-rs carries it (the text names no copyright holder).
- `kbkdf` 0.1.0-rc.1: its own directory in https://github.com/RustCrypto/KDFs holds no license file, so these are
  `hkdf/LICENSE-MIT` and `hkdf/LICENSE-APACHE` of that repository at a163784aa7d04b9496e3fb7afb56ef8fdeb35512.
- `bollard-stubs` 1.53.1-rc.29.3.1: `LICENSE` of https://github.com/fussybeaver/bollard at
  17c63341addd61d5d410a7e79444cfcf5e43a1f9, the repository whose `codegen/` publishes it.
- `drm-fourcc` 2.2.0: `LICENSE` of https://github.com/danielzfranklin/drm-fourcc-rs at
  1ad1a978a9aaa78ff0016cc7013787e4d1d68682 (its text names no copyright holder; the crate's author is Daniel
  Franklin).
- `profiling` 1.0.18: `LICENSE-MIT` and `LICENSE-APACHE` of https://github.com/aclysma/profiling at
  da44bd1c5497c665c0f00e5d70594fc14d390b4b.
- `wtransport`, `wtransport-proto` 0.7.2: `LICENSE-MIT` and `LICENSE-APACHE` of https://github.com/BiagioFesta/wtransport
  at d022a6ad5f6260cb526e06d8ca42d09516ce6440 (its tag 0.7.2).
- `httlib-huffman` 0.3.4: `LICENSE` of https://github.com/xpepermint/httlib-rs at
  90b8b8f1587e425137d08487d70dd0640fcd4d91.

Crates the Cua Driver binary links (its notices, under `cua-driver-rs-<version>/`):

- `jsonschema-regex` 0.46.10: `LICENSE` of https://github.com/Stranger6667/jsonschema at
  e9af61d618fd012a85abad5eeb3c1a7b02707db6.
- `rquickjs-core` 0.14.0: `LICENSE` of https://github.com/DelSkayn/rquickjs at c0bef90967bd91fbe12ce882b81d280db94843d0.
- `uniffi`, `uniffi_core`, `uniffi_pipeline` 0.31.0: `LICENSE` of https://github.com/mozilla/uniffi-rs
  at c31f4bc025c7a31eae56b49107f50e6d38e76bc7.
- `include_dir` 0.7.4: `LICENSE` of https://github.com/Michael-F-Bryan/include_dir at
  d742c6fffce99ee89da91b934e7ce6fb2a82680c.
- `uuid-simd`, `vsimd` 0.8.0: `LICENSE` of https://github.com/Nugine/simd at dff9585c080896a9ebdb8d6dcabce17bd69a0874.

npm packages the client's lock lists as production dependencies (the Svelte compiler's) that ship no license text,
and whose repositories hold none either: their `package.json` declares MIT, with Rich Harris as author, so the file
here is the MIT License with that author as the copyright holder.

- `is-reference` 3.0.3 (https://github.com/Rich-Harris/is-reference at 8bb053129bfabe2f6a7d7ed050159d67ebe82829).
- `locate-character` 3.0.0 (https://github.com/Rich-Harris/locate-character at
  08bc0b7674e53239083988c961057fe59f6265e2).

The Cua Driver's own texts, in `cua-driver-rs-<version>/`, are its repository's (https://github.com/trycua/cua at the
release's tag `cua-driver-rs-v<version>`, 01df64326f94f5b1f52b02d444a83382552875b0 for 0.47.13):
`LICENSE.md` (MIT, the root license, which LICENSING.md says covers Cua Driver) and
`libs/cua-driver/rust/THIRD_PARTY_NOTICES.md`, and `Inter-OFL.txt`, from
`libs/cua-driver/rust/crates/cursor-overlay/assets/`, the license of the Inter font the binary embeds there. Its `rust-crates.md` is written from that tag's
`libs/cua-driver/rust` workspace, with the crates and features its Linux release builds, by:

    cargo fetch --locked --target x86_64-unknown-linux-gnu --manifest-path CUA/libs/cua-driver/rust/Cargo.toml
    CARGO_MANIFEST=CUA/libs/cua-driver/rust/Cargo.toml scripts/notices.py cargo \
        scripts/notices/cua-driver-rs-<version>/rust-crates.md cua-driver --built-with cursor-theme-cli,cua-driver-sdk \
        --features cua-driver/portal-input

- `openh264` 0.9.10, which only a display build with the `h264` feature links (not a published image): its
  repository (https://github.com/ralfbiedert/openh264-rs at 8c6f1ccd38525d00afe9321c0c5cd90576f14cba)
  declares BSD-2-Clause, with Ralf Biedert as author, and holds no license file of its own, so the file here is the
  BSD-2-Clause text its bundled OpenH264 source carries, with that author as the copyright holder.
