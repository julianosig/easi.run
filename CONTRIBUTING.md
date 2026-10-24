# Contributing

Thanks for helping. This file covers building, testing, and how changes are reviewed. Read
[`docs/architecture.md`](docs/architecture.md) first; it explains where each piece of code lives.

## Requirements

- Linux on x86-64 with Docker Engine 28 or newer and Docker Compose v2. The stack runs only in containers, and so can
  every check: `scripts/dev/run.sh` runs a command in the development image (the Rust toolchain, Node 26, Python 3,
  and the display's build dependencies, including SVT-AV1).
- To work on the host instead:
  - the toolchain in `rust-toolchain.toml` (`rustup` installs it automatically);
  - Node.js 26 and Python 3;
  - `libxkbcommon` headers (Debian: `libxkbcommon-dev`);
  - `cmake`, `nasm`, and `curl`, which `scripts/svt-av1.sh` uses to build SVT-AV1 3.1.2 into `target/svt-av1`.
- For fuzzing: a nightly toolchain and `cargo-fuzz`.

## Checks

Checks run only where a change can have an effect. `scripts/affected.py` reads the changed files and prints the plan:

- the crates to lint and test, with every crate that depends on them;
- whether the hostile Wayland clients, the client tests, the site check (`svelte-check` and the build), and the
  license report must run;
- the images to rebuild, and the end-to-end tests that exercise those images.

Documentation changes select nothing. A path the script cannot place selects everything.

```sh
scripts/dev/run.sh scripts/check.sh          # what the working tree's changes need
scripts/dev/run.sh scripts/check.sh main     # what everything since main needs
scripts/dev/run.sh scripts/check.sh --all    # everything (what CI runs on every pull request)
```

Formatting always runs. Clippy runs with warnings denied.

The end-to-end tests drive the real containers against the bench stack ([`docs/benchmark.md`](docs/benchmark.md)
says how to start it), and they cover the stream, control, and security requirements. `scripts/e2e.sh` takes the
same arguments as `check.sh`: it rebuilds the affected images, restarts the controller so it picks them up, and runs
only the affected test targets:

```sh
ADMIN_PASSWORD='your admin password' scripts/e2e.sh      # on the host: it drives Docker and needs only cargo
```

The controller's tests against the real Docker Engine create sessions from small stand-in images, as an instance of
their own (`easirun-test`), then remove every resource of that instance (labelled `run.easi.instance` and
`run.easi.session`). `scripts/e2e.sh` runs them whenever the controller can be affected; alone, on the host (the
development image has no Docker):

```sh
CONTROLLER_DOCKER_TESTS=1 cargo test -p controller --test docker
```

The benchmark is a measurement, not a test. While iterating, run it on the fixtures a change can affect, with short
runs and few repetitions, in several CPU lanes at once; keep the full size for a published result
([`docs/benchmark.md`](docs/benchmark.md) has the method):

```sh
ADMIN_PASSWORD=... python3 bench/runner/run.py --out /tmp/bench --fixtures transform,scroll \
    --reps 0 --headless-reps 2 --seconds 20 --lanes 4
```

Fuzz every parser for SECONDS each (default 3600):

```sh
tests/fuzz/run.sh 600
```

## Working on the code

- **Security first.** Never weaken a boundary in [`docs/security.md`](docs/security.md) to make something work. A
  change that touches a boundary comes with a test that proves the boundary still holds.
- **Generic first, the app in its adapter.** What the Wayland compositor or a freedesktop standard (a portal, the
  sound server) can deliver is implemented there, so it works for any Wayland app: input, the clipboard, drag and
  drop, the file chooser, sound. What only the app knows (how Chrome writes a download, the size it knows, its
  policy) is integrated in that app's adapter, the launcher in `browser/launch/`, and never in the display, the admin
  application, or the client, which stay app-agnostic. Running another app means writing another adapter.
- **Bounds everywhere.** Every input from a client, a page, or the network is bounded and validated, and an
  unexpected input closes the connection rather than being guessed at.
- **No `unsafe`** outside `display/src/stream/av1.rs` and `display/src/stream/opus.rs` (the C encoders' FFI). The
  workspace denies it; a new use needs a stated justification and a test.
- **The protocol is closed and versioned.** A wire change updates `protocol/src/lib.rs`, its golden vectors in
  `protocol/vectors/`, and the TypeScript mirror `client/src/protocol.ts` together.
- **Performance claims are measured.** A change made for speed comes with numbers from the benchmark runner or a
  focused measurement, taken on the same host before and after.
- **Dependencies** must use permissive licenses compatible with AGPL-3.0 (MIT, Apache-2.0, BSD, ISC, Zlib, or similar).
  `scripts/licenses.py` fails on anything else. The one exception, approved by the owner, is Google Chrome in the
  browser image, so the browser is an ordinary Chrome to the sites it visits. After an image change, regenerate the SBOMs with `scripts/sbom.sh`.
- **Style.** `cargo fmt` (120 columns) and Clippy are authoritative. Comments explain why, not what. Names say what a
  thing is; avoid abbreviations.

## Updating pinned versions

Every base image is pinned by digest, and every downloaded archive by checksum.

- **Chrome.** `browser/Dockerfile` pins Google Chrome's package by URL and SHA-256, both from the `Packages` index of
  Google's signed repository (`dl.google.com/linux/chrome/deb`). Google keeps only the current version, so bump the
  pin when a build fails to fetch it. The KasmVNC baseline (`bench/kasmvnc/Dockerfile`) pins the same package by the
  same URL and SHA-256, so benchmarks compare one Chrome build: bump both Dockerfiles together. After a bump:
  - rebuild the images;
  - run the end-to-end tests, including `chromium_keeps_its_sandbox_and_blocks_internal_schemes`;
  - run the agent checks in [`docs/agent-compatibility.md`](docs/agent-compatibility.md);
  - regenerate the SBOMs.
- **Base images** (`rust`, `debian`, `distroless`, `node`, Playwright). Change the tag and its digest together, in
  every Dockerfile that uses it and in `site/deploy/release.sh` (`node:26-slim`). A new Rust version also updates
  `rust-toolchain.toml`, `rust-version` in `Cargo.toml`, and the CI container. The Playwright image is pinned in
  `bench/runner/run.py` and `docs/benchmark.md`; bump the `playwright` package in `bench/runner/package.json` (and
  its lock file) to the same version.
- **SVT-AV1.** Update `VERSION` and `SHA256` in `scripts/svt-av1.sh`, then measure AV1 encode time with
  `cargo test -p display --release speed_at_1280x800 -- --ignored --nocapture` before and after.
- **libopus.** Update `VERSION` and `SHA256` in `scripts/opus.sh` (Xiph publishes the checksums), then run the display
  tests and the `audio` end-to-end tests.

## Commits and pull requests

- One logical change per commit, in English, with an imperative subject and a conventional prefix (`feat:`, `fix:`,
  `perf:`, `refactor:`, `test:`, `docs:`, `build:`, `ci:`). The body says why.
- A pull request explains the problem, the change, and how it was verified.
- Report security problems privately, as [`SECURITY.md`](SECURITY.md) describes, never in a public issue.

## License

By contributing, you agree that your contribution is licensed under the AGPL-3.0-only, like the rest of the project,
without additional terms.
