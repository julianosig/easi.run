# Contributing

Thanks for helping. This file covers building, testing, and how changes are reviewed. Read
[`docs/architecture.md`](docs/architecture.md) first; it explains where each piece of code lives.

## Requirements

- Linux on x86-64 with Docker Engine 28 or newer and Docker Compose v2. The stack runs only in containers, and so can
  every check: `scripts/dev/run.sh` runs a command in the development image (the Rust toolchain, Node 26, Python 3,
  the display's build dependencies, including SVT-AV1 and libopus, ShellCheck, Ruff, and cargo-llvm-cov).
- To work on the host instead:
  - the toolchain in `rust-toolchain.toml` (`rustup` installs it automatically);
  - Node.js 26 and Python 3;
  - `libxkbcommon` headers (Debian: `libxkbcommon-dev`);
  - `cmake`, `nasm`, `curl`, and a C compiler, which `scripts/svt-av1.sh` and `scripts/opus.sh` use to build SVT-AV1
    into `target/svt-av1` and libopus into `target/opus`;
  - `patch` and `sha256sum`, which `scripts/third_party.sh` uses;
  - ShellCheck and Ruff, for the scripts' lints.
- For fuzzing: a nightly toolchain and `cargo-fuzz`.
- For coverage: `cargo-llvm-cov` and the toolchain's `llvm-tools-preview` component.

## Checks

Checks run only where a change can have an effect. `scripts/affected.py` reads the changed files and prints the plan:
the crates to lint and test, with every crate that depends on them; the other checks that must run; the images to
rebuild; and the end-to-end tests those images or a changed test need. Its docstring lists every kind of check.

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
(`https://localhost:9444`); it stays in the bench stack's own
`admin-state` volume, and the tests take it as `ADMIN_PASSWORD`. `scripts/e2e.sh` takes the same arguments as
`check.sh`: it rebuilds the affected images, starts the controller and the admin afresh so they pick them up, and runs
only the affected test targets, side by side (a test stops only the sessions it started), then the tests in a module
named `exclusive`, which change the admin for everyone, alone. It fails unless every test of those targets ran
exactly once:

```sh
ADMIN_PASSWORD='your admin password' scripts/e2e.sh      # on the host: Docker with Compose, cargo, Python 3, git, curl,
                                                          # Bash 5.1 or later, flock
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

## Coverage

The README's coverage badge is the line coverage of the tests CI's check job runs: every crate's unit and integration
tests (the display's also with its H.264 encoder; the ignored ones, which need Docker or the stack, do not run),
display-tests' stackless targets (`hostile` and `seccomp`), and the client's unit tests. The end-to-end tests need
the stack and are not measured, so the code only they reach counts as uncovered.

`scripts/coverage.sh` measures the Rust code with cargo-llvm-cov, without the test code (the `tests/` directories,
`tests.rs` modules, and `#[cfg(test)]` items), the benchmark crate, the vendored crate, and generated code; and the
client with c8, over every TypeScript module in `client/src` (a module no test imports counts as wholly uncovered; the
Svelte components are not measured). LLVM counts the lines that hold code, c8 every line of a module. The total is the
lines covered over the lines counted across both, rounded down; `target/coverage/rust.lcov` still holds the
`#[cfg(test)]` items' lines, which only the printed numbers and the badge leave out. CI measures it on every push to
`main` and publishes it to the `badges` branch. To measure it yourself:

```sh
scripts/dev/run.sh scripts/coverage.sh    # prints Rust's, the client's, and the total; lcov files in target/coverage/
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
  `protocol/vectors/`, and the browser's half of it in TypeScript, `client/src/protocol.ts`, together.
- **Performance claims are measured.** A change made for speed comes with numbers from the benchmark runner or a
  focused measurement, taken on the same host before and after.
- **Dependencies** must use licenses compatible with AGPL-3.0. `scripts/licenses.py` checks what the runtime images
  and the pages ship: a Rust crate or npm package must be permissive (MIT, Apache-2.0, BSD, ISC, Zlib, or similar) or
  MPL-2.0 (or, served with a page, an OFL-1.1 typeface), and a runtime image's Debian package whose copyright file
  names a GPL term must be classified by hand in its `CLASSIFIED` table with the license of what ships (an LGPL
  library linked dynamically, MPL or OFL data, for example); it fails on anything else. Google Chrome in the browser
  image is the one proprietary component, approved by the owner, so the browser is an ordinary Chrome to the sites it
  visits. After an image change, regenerate the SBOMs with `scripts/sbom.sh`. Every published image carries the license
  and notice texts of what it ships in `/usr/share/doc/easi.run` (`scripts/notices.py`, run by the image builds): a
  crate or npm package that ships none fails the build and `scripts/licenses.py`, until its repository's files are
  copied into `scripts/notices/<name>-<version>/` (its `README.md` says where each is from). A new Cua Driver release
  needs its texts there too, which that `README.md` says how to write.
- **Style.** `cargo fmt` (120 columns) and Clippy are authoritative. Comments explain why, not what. Names say what a
  thing is; avoid abbreviations.

## Design preview

The admin page runs on example data, with hot reload and no images or stack, for checking every screen's design:
`client/preview/` holds a mock backend (a Vite dev-server plugin answering `/api/*` and the viewer stream with an example
screen as lossless tiles) and the scenarios, from the first password and the second step to the session screen and the
public demo. Nothing there is part of the build: `client/src/app.css` keeps it out of Tailwind's sources, and no page
imports it.

```sh
client/preview/run.sh                     # this checkout, in the container easi-run-preview on 127.0.0.1:5181
client/preview/run.sh /path/to/worktree   # switch it to another checkout's client/ (with its own client/preview)
ssh -N -L 5181:127.0.0.1:5181 <server>    # from your machine, then open http://localhost:5181/__preview/
```

Each scenario on that index starts afresh in your browser; any address also takes `?scenario=<id>`. Without Docker,
`npm run preview:mock` in `client/` serves it on 127.0.0.1:5181.

## Updating pinned versions

Every base image is pinned by digest, every downloaded archive by checksum, and Google Chrome's repository by its key.

- **Chrome.** Google Chrome's terms do not allow redistributing it, so every build installs it itself: the `chrome`
  stage of the root `Dockerfile` fetches the current stable package from Google's signed apt repository
  (`dl.google.com/linux/chrome/deb`), whose key it trusts only by the fingerprint it names. The browser image is the
  `browser-base` target, everything Chrome runs with but Chrome (its package's dependencies, from its `Depends`
  field), which CI publishes, and the `browser` target, which adds Chrome's files to it: `bash install.sh` builds only
  that last step, on the published base. Each build fetches the repository's signed index, so a new stable rebuilds
  the browser image, and `bash install.sh` takes it to an installation; a stable whose declared dependencies differ
  from the ones the base was built for stops that step until a release brings a new base. The KasmVNC baseline installs Chrome
  from the same stage, so one build installs one Chrome in both. No cache CI writes holds Chrome: the browser and the
  baseline write none. There is no version to bump: when Google moves its repository to another key, change the
  fingerprint in that stage.
  When a build brings a new Chrome major version:
  - run the end-to-end tests, including `chromium_keeps_its_sandbox_and_blocks_internal_schemes`;
  - run the agent end-to-end tests (`tests/tests/agents.rs`), and for DevTools mode repeat the measurement in
    [`docs/agent-compatibility.md`](docs/agent-compatibility.md) (`browser_prepare` and the `browser_*` tools);
  - regenerate the SBOMs.
- **Base images** (`rust`, `debian`, `distroless`, `node`, `nginx-unprivileged`, Playwright). Change the tag and its
  digest together, in every Dockerfile that uses it and in every file `scripts/pins.py` lists in `PINNED` (and `node`
  in `client/preview/run.sh`; `debian` in `install.sh`'s `ROOT_IMAGE` as well, whose `nsenter` runs the installer's
  root steps: the new image must still have it). A new Rust
  version also updates `rust-toolchain.toml`; `scripts/pins.py` checks that each image has one tag and digest across
  the Dockerfiles and those files, and that the `rust` image's version is the toolchain's. For a
  new Playwright image, bump the `playwright` package in `bench/runner/package.json` (and its lock file) to the same
  version.
- **SVT-AV1.** Update `VERSION` and `SHA256` in `scripts/svt-av1.sh`, then measure AV1 encode time with
  `cargo test -p display --release speed_at_1280x800 -- --ignored --nocapture` before and after.
- **libopus.** Update `VERSION` and `SHA256` in `scripts/opus.sh` (Xiph publishes the checksums), then run the display
  tests and the `audio` end-to-end tests.
- **Ruff.** Update `RUFF_VERSION` and `RUFF_SHA256` in `scripts/dev/Dockerfile` (each release publishes the archive's
  `.sha256`), then run `scripts/dev/run.sh scripts/check.sh --all`.
- **cargo-llvm-cov.** Update `CARGO_LLVM_COV_VERSION` and `CARGO_LLVM_COV_SHA256` in `scripts/dev/Dockerfile` (each
  release lists its archives' SHA-256 digests), then run `scripts/dev/run.sh scripts/coverage.sh`.

## Releases

`install.sh` installs one release: a commit of the public repository and the images CI built from it, which it pulls
by digest. On every push to the public repository's `main`, once the checks, the images and the controller's tests
pass, CI's `publish` job builds the runtime images from `compose.yaml` (the browser as its Chrome-less base, checked
for any trace of Chrome) and pushes them to `ghcr.io/julianosig/easi.run-<role>` (`admin`, `controller`, `display`,
`agent`, `proxy`, `browser`), tagged with the commit and labelled with it; its summary lists their digests. That job
alone may write packages, and pull requests never run it. To release a commit `S` of the public repository:

1. push `S` to the public `main`, and wait until its CI run has passed, the `publish` job included;
2. here, run `scripts/pin.py S`: it reads each image anonymously, as an installation will, checks it against its
   digest and its labels, and writes `PINNED_COMMIT` and `PINNED_IMAGES` in `install.sh`;
3. commit that as `build: Pin the installer to S`, then publish the site (`site/deploy/release.sh`, which refuses an
   `install.sh` that does not pin a whole release).

A new package on GHCR is private: once, after the first publish, make each of the six public (the package's
**Package settings**, then **Change visibility**), or `scripts/pin.py` and every installation fail to read it.
Re-running the `publish` job pushes new images under the commit's tags: installations keep pulling the digests pinned,
and `scripts/pin.py` takes whatever the tags name when it runs.

## Commits and pull requests

- One logical change per commit, in English, with an imperative subject and a conventional prefix (`feat:`, `fix:`,
  `perf:`, `refactor:`, `test:`, `docs:`, `build:`, `ci:`). The body says why.
- A pull request explains the problem, the change, and how it was verified.

## License

By contributing, you agree that your contribution is licensed under the AGPL-3.0-only, like the rest of the project,
without additional terms.
