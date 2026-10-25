# Contributing

Thanks for helping. This file covers building, testing, and how changes are reviewed. Read
[`docs/architecture.md`](docs/architecture.md) first; it explains where each piece of code lives.

## Requirements

- Linux on x86-64 with Docker Engine 28 or newer and Docker Compose v2. The stack runs only in containers, and so can
  every check: `scripts/dev/run.sh` runs a command in the development image (the Rust toolchain, Node 26, Python 3,
  the display's build dependencies, including SVT-AV1 and libopus, ShellCheck, and Ruff).
- To work on the host instead:
  - the toolchain in `rust-toolchain.toml` (`rustup` installs it automatically);
  - Node.js 26 and Python 3;
  - `libxkbcommon` headers (Debian: `libxkbcommon-dev`);
  - `cmake`, `nasm`, `curl`, and a C compiler, which `scripts/svt-av1.sh` and `scripts/opus.sh` use to build SVT-AV1
    3.1.2 into `target/svt-av1` and libopus 1.5.2 into `target/opus`;
  - `patch` and `sha256sum`, which `scripts/third_party.sh` uses;
  - ShellCheck and Ruff, for the scripts' lints.
- For fuzzing: a nightly toolchain and `cargo-fuzz`.

## Checks

Checks run only where a change can have an effect. `scripts/affected.py` reads the changed files and prints the plan:

- the crates to lint and test, with every crate that depends on them;
- whether the stackless integration tests (the hostile Wayland clients and the browser seccomp profile's rules), the
  client tests, the site check (`svelte-check`, the build, and its tool list against `admin/src/tools.json`), the
  license report, the fuzz targets' lints, the public demo's host lists (`scripts/demo.py`), the scripts' lints
  (ShellCheck and Ruff), the pinned versions (`scripts/pins.py`), the vendored crate against its release
  (`scripts/third_party.sh`), and the egress Compose overlay must run;
- the images to rebuild, and the end-to-end tests those images or a changed test need.

Documentation changes select nothing, except the generated license report (`docs/licenses.md` and `docs/sbom/`),
which selects its check, `docs/benchmark.md`, which pins the Playwright image and selects the pins check, and the
vendored crate's (anything under `third_party/`), which selects everything. A path the
script cannot place selects everything.

```sh
scripts/dev/run.sh scripts/check.sh          # what the working tree's changes need
scripts/dev/run.sh scripts/check.sh main     # what everything since main needs
scripts/dev/run.sh scripts/check.sh --all    # every check without Docker or the stack (what CI's check job runs)
```

Formatting runs on every plan. Clippy runs with warnings denied.

The end-to-end tests run on the host, against the bench stack ([`docs/benchmark.md`](docs/benchmark.md) says how to
start it): they drive the real containers and cover the stream, control, and security requirements. The bench stack
is an installation of its own, beside any installed easi.run on the same host: its Compose project
(`easi-run-bench`), controller instance (`easirun-bench`), image tags (`easi-run-bench/*:dev`), volumes and port
(`https://localhost:9444`) are its own, so neither it nor the tests rebuild, recreate or stop an installation's
containers or sessions. Set its admin's password first, from a script with `admin init`
(`docker compose -p easi-run-bench -f compose.yaml -f compose.bench.yaml run --rm -T --no-deps init < password-file`,
a file of one line of 12 to 1024 characters, once the images are built) or in the admin's first screen
(`docker exec easi-run-bench-admin-1 admin setup` prints its link); it stays in the bench stack's own
`admin-state` volume, and the tests take it as `ADMIN_PASSWORD`. `scripts/e2e.sh` takes the same arguments as
`check.sh`: it rebuilds the affected images, starts the controller and the admin afresh so they pick them up, and runs
only the affected test targets:

```sh
ADMIN_PASSWORD='your admin password' scripts/e2e.sh      # on the host: Docker with Compose, cargo, Python 3, git, curl
```

The controller's tests against the real Docker Engine create sessions from small stand-in images, as an instance of
their own (`easirun-test`), then remove every resource of that instance (labelled `run.easi.instance` and
`run.easi.session`). `scripts/e2e.sh` runs them whenever the controller can be affected; alone, on the host (the
development image has no Docker):

```sh
cargo test -p controller --test docker -- --ignored
```

The benchmark is a measurement, not a test. While iterating, run it on the fixtures a change can affect, with short
runs and few repetitions, in several CPU lanes at once; keep the full size for a published result
([`docs/benchmark.md`](docs/benchmark.md) has the method):

```sh
ADMIN_PASSWORD=... python3 bench/runner/run.py --out /tmp/bench --fixtures transform,scroll \
    --reps 0 --headless-reps 2 --seconds 20 --lanes 3    # at most the lanes the stack has (3, as docs/benchmark.md starts it)
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
- **No `unsafe`** outside `display/src/stream/av1.rs` and `display/src/stream/opus.rs` (the C encoders' FFI), and the
  empty `unsafe extern` block through which the `microphone_packet` fuzz target links libopus. Both workspaces, the
  main one and `tests/fuzz`, deny it; a new use needs a stated justification and a test.
- **The protocol is closed and versioned.** A wire change updates `protocol/src/lib.rs`, its golden vectors in
  `protocol/vectors/`, and the TypeScript mirror `client/src/protocol.ts` together.
- **Performance claims are measured.** A change made for speed comes with numbers from the benchmark runner or a
  focused measurement, taken on the same host before and after.
- **Dependencies** must use licenses compatible with AGPL-3.0. `scripts/licenses.py` checks what the runtime images
  and the pages ship: a Rust crate or npm package must be permissive (MIT, Apache-2.0, BSD, ISC, Zlib, or similar) or
  MPL-2.0 (or, served with a page, an OFL-1.1 typeface), and a runtime image's Debian package whose copyright file
  names a GPL term must be classified by hand in its `CLASSIFIED` table with the license of what ships (an LGPL
  library linked dynamically, MPL or OFL data, for example); it fails on anything else. Google Chrome in the browser
  image is the one proprietary component, approved by the owner, so the browser is an ordinary Chrome to the sites it
  visits. After an image change, regenerate the SBOMs with `scripts/sbom.sh`.
- **Style.** `cargo fmt` (120 columns) and Clippy are authoritative. Comments explain why, not what. Names say what a
  thing is; avoid abbreviations.

## Updating pinned versions

Every base image is pinned by digest, every downloaded archive by checksum, and Google Chrome's repository by its key.

- **Chrome.** Google Chrome's terms do not allow redistributing it, so every build installs it itself: the `chrome`
  stage of `browser/Dockerfile` fetches the current stable package from Google's signed apt repository
  (`dl.google.com/linux/chrome/deb`), whose key it trusts only by the fingerprint it names. Each build fetches the
  repository's signed index, so a new stable rebuilds the browser image, and `bash install.sh` takes it to an
  installation. The KasmVNC baseline (`bench/kasmvnc/Dockerfile`) holds the same stage, byte for byte, so one build
  installs one Chrome in both (`scripts/pins.py` checks it). There is no version to bump: when Google moves its
  repository to another key, change the fingerprint in both stages. When a build brings a new Chrome major version:
  - run the end-to-end tests, including `chromium_keeps_its_sandbox_and_blocks_internal_schemes`;
  - run the agent checks in [`docs/agent-compatibility.md`](docs/agent-compatibility.md);
  - regenerate the SBOMs.
- **Base images** (`rust`, `debian`, `distroless`, `node`, `nginx-unprivileged`, Playwright). Change the tag and its
  digest together, in every Dockerfile that uses it, in `compose.bench.yaml` and `tests/tests/security.rs` (`debian`),
  in `demo/compose.yaml` (`nginx-unprivileged`), and in `site/deploy/release.sh` (`node:26-slim`). A new Rust version
  also updates `rust-toolchain.toml` and `rust-version` in `Cargo.toml`; `scripts/pins.py` checks that each image has
  one tag and digest across the Dockerfiles and the files in its `PINNED`, and that the three Rust versions agree. The
  Playwright image is pinned in `bench/runner/run.py` and `docs/benchmark.md`; bump the `playwright` package in
  `bench/runner/package.json` (and its lock file) to the same version.
- **SVT-AV1.** Update `VERSION` and `SHA256` in `scripts/svt-av1.sh`, then measure AV1 encode time with
  `cargo test -p display --release speed_at_1280x800 -- --ignored --nocapture` before and after.
- **libopus.** Update `VERSION` and `SHA256` in `scripts/opus.sh` (Xiph publishes the checksums), then run the display
  tests and the `audio` end-to-end tests.
- **Ruff.** Update `RUFF_VERSION` and `RUFF_SHA256` in `scripts/dev/Dockerfile` (each release publishes the archive's
  `.sha256`), then run `scripts/dev/run.sh scripts/check.sh --all`.

## Commits and pull requests

- One logical change per commit, in English, with an imperative subject and a conventional prefix (`feat:`, `fix:`,
  `perf:`, `refactor:`, `test:`, `docs:`, `build:`, `ci:`). The body says why.
- A pull request explains the problem, the change, and how it was verified.
- Report security problems privately, as [`SECURITY.md`](SECURITY.md) describes, never in a public issue.

## License

By contributing, you agree that your contribution is licensed under the AGPL-3.0-only, like the rest of the project,
without additional terms.
