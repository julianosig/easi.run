# Fallback license texts

The license files of crates whose published package ships none, taken from their repositories, which
`scripts/notices.py` adds for the version each directory names (`<crate>-<version>`): a new version
has no fallback until its files are checked and copied here again.

- `base64urlsafedata`, `webauthn-attestation-ca`, `webauthn-rs`, `webauthn-rs-core`, `webauthn-rs-proto`
  0.5.5: `LICENSE.md` of https://github.com/kanidm/webauthn-rs at be696b79800bd1953df78e87d0215571733cc26f.
- `bollard-stubs` 1.53.1-rc.29.3.1: `LICENSE` of https://github.com/fussybeaver/bollard at
  17c63341addd61d5d410a7e79444cfcf5e43a1f9, the repository whose `codegen/` publishes it.
- `drm-fourcc` 2.2.0: `LICENSE` of https://github.com/danielzfranklin/drm-fourcc-rs at
  1ad1a978a9aaa78ff0016cc7013787e4d1d68682 (its text names no copyright holder; the crate's author is Daniel
  Franklin).
- `profiling` 1.0.18: `LICENSE-MIT` and `LICENSE-APACHE` of https://github.com/aclysma/profiling at
  da44bd1c5497c665c0f00e5d70594fc14d390b4b.

Crates the Cua Driver binary links (its notices, under `cua-driver-rs-<version>/`):

- `jsonschema-regex` 0.46.10: `LICENSE` of https://github.com/Stranger6667/jsonschema at
  e9af61d618fd012a85abad5eeb3c1a7b02707db6.
- `rquickjs-core` 0.14.0: `LICENSE` of https://github.com/DelSkayn/rquickjs at c0bef90967bd91fbe12ce882b81d280db94843d0.
- `uniffi`, `uniffi_core`, `uniffi_meta`, `uniffi_pipeline` 0.31.0: `LICENSE` of https://github.com/mozilla/uniffi-rs
  at c31f4bc025c7a31eae56b49107f50e6d38e76bc7.
- `uuid-simd`, `vsimd` 0.8.0: `LICENSE` of https://github.com/Nugine/simd at dff9585c080896a9ebdb8d6dcabce17bd69a0874.

npm packages the client's lock lists as production dependencies (the Svelte compiler's) that ship no license text,
and whose repositories hold none either: their `package.json` declares MIT, with Rich Harris as author, so the file
here is the MIT License with that author as the copyright holder.

- `is-reference` 3.0.3 (https://github.com/Rich-Harris/is-reference at 8bb053129bfabe2f6a7d7ed050159d67ebe82829).
- `locate-character` 3.0.0 (https://github.com/Rich-Harris/locate-character at
  08bc0b7674e53239083988c961057fe59f6265e2).

The Cua Driver's own texts, in `cua-driver-rs-<version>/`, are its repository's (https://github.com/trycua/cua at the
release's tag `cua-driver-rs-v<version>`, 6d6c734c127025fb39ad5d585a10382808b39dc1 for 0.36.0):
`LICENSE.md` (MIT, the root license, which LICENSING.md says covers Cua Driver) and
`libs/cua-driver/rust/THIRD_PARTY_NOTICES.md`, and `Inter-OFL.txt`, from
`libs/cua-driver/rust/crates/cursor-overlay/assets/`, the license of the Inter font the binary embeds there. Its `rust-crates.md` is written from that tag's
`libs/cua-driver/rust` workspace, with the crates and features its Linux release builds, by:

    cargo fetch --locked --target x86_64-unknown-linux-gnu --manifest-path CUA/libs/cua-driver/rust/Cargo.toml
    CARGO_MANIFEST=CUA/libs/cua-driver/rust/Cargo.toml scripts/notices.py cargo \
        scripts/notices/cua-driver-rs-<version>/rust-crates.md cua-driver --built-with cursor-theme-cli,cua-driver-sdk \
        --features cua-driver/portal-input

- `openh264` 0.9.9, which only a display build with the `h264` feature links (not a published image): its
  repository (https://github.com/ralfbiedert/openh264-rs at 5f3637e2a2e4eead27f21816c050a6a9064ad52e)
  declares BSD-2-Clause, with Ralf Biedert as author, and holds no license file of its own, so the file here is the
  BSD-2-Clause text its bundled OpenH264 source carries, with that author as the copyright holder.
