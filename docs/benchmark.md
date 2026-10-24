# Benchmark

How easi.run is measured against KasmVNC, and what the measurements show. The targets are in
[design §8](design.md#8-acceptance-the-benchmark-against-kasmvnc).

## Method

**Fairness.**

- One host and the same browser in both systems: Google Chrome (the version is in each result's machine line, in `summary.md` and `summary.json`) with the
  browser launcher's flags, plus window geometry for KasmVNC's X server, in kiosk mode on both, so the page alone fills
  the screen (easi.run's benchmark sessions ask for kiosk mode, which only the bench profile grants; its everyday
  window has Chrome's tab strip and toolbar).
- A 1280x800 output.
- The same total CPU for the screen path in a lane (`--lanes N`, N of 1 or more, as every published result uses).
  easi.run gets `browser` 1 CPU (the bench profile sets it; the default is 2, see [Browser limits](#browser-limits))
  and `display` 1 CPU (2 outside a lane, as in every session), each a quota per 10 ms period; KasmVNC gets one
  container with 2 CPUs, a quota per Docker's default 100 ms period.
- The fixtures come over HTTPS through each system's own proxy.
- Each run is measured for its full window after a 5-second warm-up.

**Clients.** Both systems are measured the same way, from the canvas a person sees, in Playwright's Chromium:

- **Presented frames:** distinct time stamps decoded from the drawn frames. Each fixture draws its time into a strip
  in its top-left corner.
- **Capture-to-draw:** the client's clock minus the drawn stamp. Both ends share the host clock, so the offset is zero
  and the bound is the stamp's 1 ms resolution.
- **Input-to-draw:** from a key press to the typing fixture's square flipping in a drawn frame.
- **Quality:** SSIM of frames captured every 1.5 s against each fixture's exact reference frame. Capture starts with
  the warm-up and stops at 30 frames, so it covers about the first 40 s of a 60 s window.
- **CPU:** each container's cgroup usage over the exact measurement window.

KasmVNC uses its own web client, with its dynamic quality set to 7–9 and a 60 fps target.

**Fixtures.** They are served by a host that exists only in the `bench` profile:

| Fixture | What it measures |
|---|---|
| `article` | a static article: traffic after the last damage |
| `scroll` | smooth scrolling through a long page |
| `transform` | full-motion composited animation, which Chrome sustains at 60 fps on one CPU |
| `animation` | a full-screen repaint every frame (stress, no target: Chrome alone reaches about 30 fps on one CPU) |
| `video` | a 1080p video |
| `text-scroll` | text and figures scrolling, with exact reference frames: motion bitrate and quality |
| `quality` | flat blocks that all change every frame (stress, no target: 4:2:0 chroma caps a video codec's SSIM below 0.97) |
| `typing` | keys typed under control: input-to-draw |

**Lanes.** Runs can be measured several at a time, each in its own CPU lane (`bench/runner/lanes.py`). A lane owns
whole physical cores, both SMT siblings, on one NUMA node:

- one core for the browser, one for the display (KasmVNC gets both);
- two more cores for the measuring client;
- the session's memory on the same node.

The controller applies the placement when it creates the session, and the first two cores of each node stay with the
host. Within each block (one fixture and repetition), the systems are shuffled across lanes so contention falls on
all of them alike, and SSIM is computed after the last measurement. Each run records the host's load and its lane's
CPU frequency at the start and end of its window.

A lane count is trusted only after a calibration: the same runs measured one lane at a time and N lanes at once
must agree on every metric within the repetitions' own spread. The calibration on 2026-10-19 used every fixture and
system, headless, 2 x 20 s each, and compared each median with the one-lane run:

| Lanes | presented fps | capture-to-draw p50 | capture-to-draw p95 | screen-path CPU |
|---|---|---|---|---|
| 3 | -0.4% (abs 1.1%) | +2.7% (abs 3.4%) | +1.1% (abs 2.1%) | +0.7% (abs 1.1%) |
| 6 | -0.3% (abs 2.7%) | +6.3% (abs 7.1%) | +6.7% (abs 7.0%) | +7.7% (abs 7.7%) |

The values are the median difference and, in parentheses, the median absolute difference. Two repetitions of the
same run differed by 2% at the median and by up to 29%. Three lanes stay within that spread; six lanes shift latency
and CPU one way on nearly every fixture, which is contention rather than noise. Published results use three lanes.

**Phase noise.** The page's frames and the client's animation frames come from two independent 60 Hz clocks. Each run
therefore waits a different share of a frame for the client's next animation frame, and a run's capture-to-draw
median can move by up to about 15 ms from one run to the next. Medians over a fixture's completed runs (five per
system, fewer where a run failed) are what the targets compare.

## Running it

```sh
export DOCKER_GID=$(stat -c %g /var/run/docker.sock) BENCH_LANES="$(python3 bench/runner/lanes.py 3)"
docker compose -f compose.yaml -f compose.bench.yaml up -d --build
docker build -t easi-run/tools:dev bench/tools        # ffmpeg, for SSIM
docker run --rm -u $(id -u) -e HOME=/tmp -v $PWD/bench/runner:/w -w /w \
    mcr.microsoft.com/playwright:v1.61.1-noble@sha256:5b8f294aff9041b7191c34a4bab3ac270157a28774d4b0660e9743297b697e48 npm ci
ADMIN_PASSWORD=... python3 bench/runner/run.py --out bench/results/<date> --lanes 3
```

`ADMIN_PASSWORD` is the admin's password, set once with `install.sh` or `admin init` (see
[CONTRIBUTING](../CONTRIBUTING.md)).
`run.py --help` lists the options. `--lanes 0` measures one run at a time, without placement.

## Results

Measured on 2026-10-21 on an Intel Xeon Platinum 8160 host (2 sockets, 96 logical CPUs, no GPU), shared with other
workloads (load average 10 at the median of the run windows, 21 at most), under Debian 13 (Linux 6.12) and Docker 29.9.
The client was Playwright 1.61.1's Chromium, headless and headed under Xvfb without a GPU. There were 5 headless and
5 headed runs of 60 s per system and fixture, in three lanes; two runs in 160 failed to load the benchmark page
(`net::ERR_NETWORK_CHANGED`) and are excluded. Raw runs, with every latency sample, are in
[`bench/results/2026-10-21/`](../bench/results/2026-10-21/). easi.run ran the published images, which encode
motion only in AV1.

Since [the 2026-10-20 run](../bench/results/2026-10-20/), both systems run Google Chrome instead of Chromium, and
easi.run's browser container carries the browser's sound: the screen path's end-of-window memory is 35–40 MiB higher
for KasmVNC and 45–52 MiB for easi.run (headless medians per fixture). easi.run's containers' CPU quota is measured
over 10 ms periods, and its video covers only the part of the screen that moves (see
[what was tried](#what-limits-capture-to-draw)).

### Targets

Medians of the headless runs that completed (five per fixture, four on `transform`) unless noted.

| Metric | Target | easi.run | KasmVNC | Verdict |
|---|---|---|---|---|
| Full-motion presented fps (`transform`) | ≥ 55 and ≥ KasmVNC | 37.7 (headed 42.4) | 24.4 | **fail** at 55; pass against KasmVNC |
| Smooth scroll presented fps (`scroll`) | ≥ 55 | 54.9 (headed 55.3) | 24.4 | **fail** by 0.1 headless; pass headed |
| Motion bitrate at equal or better SSIM (`text-scroll`) | ≤ 1/3 of KasmVNC | 1,862 kbit/s, SSIM 0.990 | 16,433 kbit/s, SSIM 0.977 | pass (11%) |
| Static page media traffic (`article`) | 0 B/s after the last damage | 0 | 0 | pass |
| Static text | bit-exact | lossless records of compositor pixels | — | pass (`each_rectangle_takes_its_smallest_record_and_round_trips`) |
| Capture-to-draw, `scroll` | p50 ≤ 50 ms, p95 ≤ 100 ms | 47 / 65 | 83 / 100 | pass |
| Capture-to-draw, `text-scroll` | p50 ≤ 50 ms, p95 ≤ 100 ms | 43 / 60 | 83 / 87 | pass |
| Capture-to-draw, `video` | p50 ≤ 50 ms, p95 ≤ 100 ms | 63 / 100 | 83 / 108 | **fail** at p50 |
| Capture-to-draw, `transform` | p50 ≤ 50 ms, p95 ≤ 100 ms | 68 / 114 | 100 / 116 | **fail** |
| Input-to-draw (`typing`) | p50 ≤ 60 ms, p95 ≤ 120 ms | 28 / 37 | 69 / 82 | pass |
| `display` memory | ≤ 256 MiB | peak 75 MiB (the highest of any run) | — | pass |
| `display` image size | reported | 105 MB (the build of 2026-10-24) | | |

### Every fixture (headless medians)

CPU and memory cover the whole screen path: `browser` and `display` for easi.run, the one container for KasmVNC.
Memory is the median over the runs of the containers' summed peaks.

| Fixture | System | fps | kbit/s | Capture-to-draw p50/p95 ms | CPU | fps per CPU | Memory MiB | SSIM |
|---|---|---|---|---|---|---|---|---|
| scroll | easi.run | 54.9 | 1,751 | 47 / 65 | 1.03 | 52.8 | 325 | |
| scroll | KasmVNC | 24.4 | 12,441 | 83 / 100 | 0.90 | 26.8 | 271 | |
| transform | easi.run | 37.7 | 1,957 | 68 / 114 | 1.56 | 24.2 | 330 | |
| transform | KasmVNC | 24.4 | 25,425 | 100 / 116 | 1.47 | 16.5 | 277 | |
| video (1080p) | easi.run | 36.2 | 2,068 | 63 / 100 | 1.51 | 23.8 | 376 | |
| video (1080p) | KasmVNC | 24.4 | 18,130 | 83 / 108 | 1.77 | 13.8 | 317 | |
| text-scroll | easi.run | 55.2 | 1,862 | 43 / 60 | 1.08 | 50.7 | 363 | 0.990 |
| text-scroll | KasmVNC | 24.3 | 16,433 | 83 / 87 | 1.13 | 21.5 | 306 | 0.977 |
| animation (stress) | easi.run | 25.5 | 2,186 | 78 / 152 | 1.80 | 14.2 | 331 | |
| animation (stress) | KasmVNC | 18.5 | 5,202 | 132 / 333 | 1.90 | 9.7 | 300 | |
| quality (stress) | easi.run | 53.5 | 1,564 | 40 / 54 | 0.92 | 57.8 | 319 | 1.000 |
| quality (stress) | KasmVNC | 24.3 | 1,740 | 67 / 84 | 0.82 | 29.3 | 312 | 0.993 |
| article | easi.run | – | 0 | – | 0.01 | – | 259 | |
| article | KasmVNC | – | 0 | – | 0.01 | – | 247 | |
| typing | easi.run | – | 128 | input 28 / 37 | 0.12 | – | 257 | |
| typing | KasmVNC | – | 142 | input 69 / 82 | 0.13 | – | 240 | |

On every motion fixture easi.run presents more frames, sends fewer bits, and draws sooner; on the two fixtures
with reference frames, its SSIM is higher too. KasmVNC uses less memory on every fixture, and less CPU on `scroll`,
`transform`, and `quality`, where it presents less than half, two thirds, and less than half the frames:

- **Memory.** A motion episode starts the SVT-AV1 encoder, whose state takes about 34 MiB in steady state and 43 MiB at
  its peak, and keeps it until the episode ends. KasmVNC encodes stateless JPEG and WebP rectangles. The gap is 31–59 MiB
  of the screen path's 271–376 MiB in video motion, 7 MiB on `quality`, which stays lossless. On a static or typed page
  it is 12–17 MiB: the display process (16 MiB on `article`) and the browser's sound server (about 7 MiB) run beside
  the same Chrome as KasmVNC's.
- **CPU.** Each extra frame costs encoding. Per presented frame, easi.run uses less CPU on every motion fixture (fps
  per CPU above).

KasmVNC's headed client leaves large regions of its canvas unpainted on this GPU-less host, so its headed SSIM
(0.23–0.29) is not a quality measurement. The comparison uses the headless runs, where both clients draw correctly.

### Browser limits

The browser's CPU and memory limits are configurable (`BROWSER_CPUS`, `BROWSER_MEMORY_MIB` in the compose environment).
`display` and the browser keep their threads on as many neighboring physical cores of one socket as their CPU quota
rounded up (see the architecture's affinity note), so a larger quota also gets the cores to run on. The defaults come
from a sweep on 2026-10-20: headless, 2 runs of 20 s per point, outside lanes. Presented fps, except YouTube, where it
is the client's draws per second of an embedded 1080p60 video (the page has no stamp); CPU is the whole screen path's
on `animation`:

| Browser CPUs | Display CPUs (encoder threads) | `animation` | `scroll` | `transform` | `video` | YouTube | CPU on `animation` |
|---|---|---|---|---|---|---|---|
| 1 | 1 (1) | 27.0 | 56.9 | 39.6 | 41.9 | 36.4 | 1.71 |
| 1.5 | 1 (1) | 28.3 | 57.2 | 38.7 | 40.2 | 35.0 | 2.03 |
| 2 | 1 (1) | 32.2 | 57.2 | 40.1 | 40.9 | 35.4 | 2.38 |
| 3 | 1 (1) | 35.0 | 56.4 | 39.5 | 40.8 | 34.5 | 3.04 |
| 1.5 | 2 (2) | 29.7 | 55.0 | 44.4 | 42.6 | 37.0 | 2.47 |
| 1.5 | 3 (3) | 28.1 | 57.6 | 42.5 | 42.0 | 36.9 | 2.67 |

Only the full-screen repaint follows the browser's CPUs. Going to 2 CPUs is the step with the largest marginal gain
(3.9 fps for 0.35 CPU more over 1.5; 3 CPUs add 2.8 fps for 0.66 CPU more), though 1 CPU still delivers the most fps per
CPU consumed (15.8 against 13.5). A second encoder thread on a second display core lifts `transform`
by 15% (38.7 to 44.4 fps) and YouTube by 6% (35.0 to 37.0) over the same 1.5 browser CPUs, but costs 0.44 CPU more on
`animation` and about 20 MiB in every motion episode, so a live stream keeps one encoder thread. Outside a bench lane
the display has 2 CPUs, the second for a recording's encoder beside the live stream ([topology](topology.md)).
Neither side's CPU moves `scroll`, `video`, or YouTube, whose rates are set elsewhere in the pipeline.
Memory is a ceiling: a YouTube video takes about 270 MiB of the browser's 2 GiB.

### What limits capture-to-draw

The time from the page drawing a frame to the client drawing it splits into four stages. The stages come from the
client's statistics, and the Chrome part is the remainder:

| Stage | Typical |
|---|---|
| Chrome, from the page's frame to the compositor's composition | 19–27 ms, one frame more (34–38 ms) in two `transform` runs; 35–39 ms on `animation` |
| Encoding | 7–9 ms (`scroll`, `text-scroll`, `video`), 11–16 ms (`animation`, `transform`) |
| Encoder pacing, relay, and network to the client | 4–7 ms (`scroll`, `text-scroll`), 8–12 ms (`video`, `animation`, `transform`) |
| Client: decode, conversion, and the wait for its next animation frame | 10–12 ms (`scroll`, `text-scroll`), 16–20 ms (`animation`, `transform`), 20–22 ms (`video`) |

Each range spans the completed runs of 2026-10-21, headless and headed; a run's Chrome part is its capture-to-draw
median minus its median from composition to the client's draw.

- **Chrome's pipeline.** In the bench profile Chrome has one CPU and no GPU. Each session settles at a pipeline
  depth that holds for the whole session: 19–27 ms from the page's frame to composition in most sessions (36 of the
  38 completed runs off `animation` on 2026-10-21), one frame more (34–38 ms) in the rest. Anything that makes
  Chrome's frame timing more regular raises presented fps to about 60 but makes the deeper sessions more common. Every run records the host's load at the start and end of its window (`host` in the
  raw runs).
- **The display's one CPU in a lane.** On `transform` and `video`, the medians above 50 ms come mostly after
  composition: AV1 encoding, encoder pacing and relay, and the client's decode take 14–21 ms more than on `scroll`
  (headless). In the [browser-limits sweep](#browser-limits), a second display CPU with a second encoder thread
  lifted `transform` from 38.7 to 44.4 fps, at 0.44 CPU more on `animation` and about 20 MiB more.
- **What was tried and measured.**

  | Change | Effect |
  |---|---|
  | `TCP_NODELAY` on viewer connections | p95 −8% to −21%; kept |
  | Chromium's `WaylandExternalBeginFrameSource` | fps up to about 60, but `transform` latency +12 ms (one more frame in Chromium's pipeline) over five 60 s runs; reverted |
  | `--disable-gpu-vsync` | no effect |
  | `--disable-frame-rate-limit` | fps −22% to −49% |
  | A wider encode slack | within noise |
  | Frame callbacks delayed, or held to a fixed 60 Hz tick of any phase | worse: the tick added about 20 ms |
  | Drawing video frames on arrival rather than at the animation frame | worse |
  | With the `h264` feature, Auto leaving AV1 when the client presents under 80% of the frames | `video` 42 → 52 fps, p50 58 → 52 ms; kept |
  | Presentation times on a steady 60 Hz grid | fps about 60, but deeper sessions more common; part of the gain came from the client sharing the host's clock, which a remote viewer would not; rejected |
  | Holding frame callbacks one interval every 5 s | worse: some sessions went two frames deeper |
  | Two encoder threads on the display's SMT pair | encode slower (7.6–11 ms) and arrival 3 ms later |
  | SVT-AV1 screen-content detection instead of forced on | no encode gain, text SSIM 0.989 → 0.984 |
  | SVT-AV1 preset 13; OpenH264 without adaptive quantization and background detection | no encode gain (8.9 → 9.9 ms, 9.0 → 9.5 ms on `transform`) |
  | Other encoders, on one CPU over 120 frames of `text-scroll` and `quality` at 2.5 Mbit/s (FFmpeg) | libaom realtime (speed 10/11) 10.7–23 ms per frame and about 55 MiB; libvpx VP9 realtime 12–25 ms and about 100 MiB (4:4:4: 17–22 ms, 140 MiB); none reached SVT-AV1's 6.5–10 ms and 35–44 MiB or its SSIM; SVT-AV1 kept |
  | Per-rectangle records (solid, palette of up to 256 colors, RGB) in one deflate stream per message, instead of deflated RGB per tile | `quality` lossless at 53 fps and 1.6 Mbit/s (SSIM 1.000, against 0.943 in AV1 before); kept |
  | Merging neighboring changed rectangles before encoding | merged rectangles fell out of the palette: 20 KB per `quality` frame against 1.6 KB unmerged; removed |
  | Motion tried lossless first, moving to AV1 when the records exceed 1.5 frame budgets or their median encode 60% of the frame interval | flat content stays exact in motion; rich content still reaches AV1 within the episode's first frames; kept |
  | AV1 over the bounding box, in whole tiles, of the tiles whose pixels changed in the last 500 ms, lossless tiles around it | no traces of an earlier picture around a playing video; kept |
  | Finding changed tiles by hashing each damaged tile instead of comparing and copying the frame; the video region read in place | encode 16.8 → 13.8 ms on `transform` and 9.5 → 7.9 ms on `video` (medians of each benchmark's 9–10 completed runs, against the same benchmark before the change), `transform` 38.1 → 39.1 fps, display memory 4–8 MiB less; kept |
  | Containers' CPU quota per 10 ms period instead of Docker's 100 ms | a page that keeps the browser's CPU busy no longer pauses the container for up to 80 ms of each period, which broke its sound; `video` 42.0 → 38.8 fps (headed, 3 x 30 s); kept for the sound |

  For frame latency alone, CFS throttling was measured and ruled out (one throttled period in 101, at 100 ms periods).
