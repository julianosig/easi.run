# Agent compatibility

Measured 2026-10-18 on one x86-64 host (Linux 6.12, Docker 29.9, Yama `ptrace_scope=1`) with Cua Driver
`cua-driver-rs` 0.36.0, Debian Chromium 155.0.8059.79 (Wayland, kiosk, software rendering), and this repository's
compositor. The probes ran in throwaway containers; the hardened topology is proven again by the security tests
(`tests/tests/security.rs`). The browser image has since moved to Google Chrome in an ordinary window (kiosk only
with `BROWSER_KIOSK=1`), and the pinned driver to `cua-driver-rs` 0.49.8, built from its source (commit
`040698e196d6a37d14703dc56157242b8b4c84a3`) with one change, below: the probes were not run again with them, and the
agents end-to-end tests (`tests/tests/agents.rs`) drive that browser through that driver's desktop tools.

That driver is compiled once for each version, commit and patch (the root Dockerfile's `driver` target) and published as
`ghcr.io/julianosig/easi.run-driver`, with the patch and the three beside the binary (`/cua/`); the agent image takes
it from there by digest, its build stops unless the three are this repository's, and `--build-arg CUA_DRIVER=driver`
compiles it in place ([`CONTRIBUTING.md`](../CONTRIBUTING.md), Releases).

## What the driver binds

`CUA_DRIVER_RS_ENABLE_WAYLAND=1` selects its native Wayland backend. On a compositor it does not recognize, it refuses
window-targeted input and the page-changing `browser_*` tools unless the compositor attests window identity. The
compositor provides that through the driver's own route, the `cua-inject v1` control socket (`CUA_INJECT_SOCKET`),
plus these globals, offered on the privileged socket only:

| Global | Version | Used for |
|---|---|---|
| `zwlr_foreign_toplevel_manager_v1` | 3 | window listing joined to AT-SPI by title; without it the driver falls back to Chrome's several AT-SPI top-levels and cannot bind a tab |
| `ext_foreign_toplevel_list_v1` | 1 | fallback listing |
| `zwlr_screencopy_manager_v1` | 3 | `get_desktop_state` |
| `zwlr_virtual_pointer_manager_v1` | 2 | desktop-scope pointer input |
| `zwp_virtual_keyboard_manager_v1` | 1 | desktop-scope keys |

The browser's window is maximized over the whole screen, tab strip included, so `get_desktop_state` shows the
window's pixels.

## What the admin does around the driver

Five things the admin does for the driver under this compositor (`admin/src/assist.rs`; the pause read in the
pinned driver's source, the others measured with 0.36.0, whose refusals the pinned driver's source still holds).
All but the pause go on the display's injection link: one `cua-inject v1` line on a stream connection of its own
(`Link::Inject`), served as the agent's own socket serves it, refused while a person holds control, and sent only
within the agent's call, which holds the agent's link, so a takeover waits for it.

- **Text beyond ASCII.** The driver's injection client types only printable ASCII, tab and newline (`cua-compositor
  cannot type '—'`). A window-targeted `type_text` of other text goes to the driver with no text, so the driver
  targets it as any (the element focused, or the point clicked, the window's binding checked; the display takes a `t`
  of no text), and the display then types the text with its `u` line, the way it types a person's: a character the
  keymap lacks with the spare key. The `u` line names the same target the driver's `t` does (the process, which the
  display holds to the window the driver's `t` bound it to, refusing one that replaced it), so a window mapped in between gets none of it;
  a call that names no process, nor an element of a snapshot read of one, is left to the driver. The agent gets the driver's answer, counting the characters typed. At most a person's
  text message (1,024 bytes) per call: a longer text is refused (`text_too_long`) before the driver is asked. Text in `run_steps` and `run_script` still reaches the driver's own refusal.
- **A pause between characters.** The driver applies `type_text`'s `delay_ms` only when it types into the focused
  application; into a window it sends the whole text to the display at once. A window-targeted `type_text` that asks
  a pause is refused (`delay_unsupported`) before the driver is asked, rather than typed at once as if the pause had
  been kept. A `type_text` step inside `run_steps` or `run_script` still reaches the driver alone, which types a
  window's text at once whatever pause the step asks.
- **A click by element that has no action here.** For an element with no press (a menu item that only selects), the
  driver resolves the element, then falls back to an X11 pointer, which the agent's container has none of (`X11
  connection failed`). The admin clicks the centre of the element's frame in the last snapshot it relayed (read with
  `tree_format` `elements` or `both`: the default Markdown carries no frames), and answers as the driver's pixel click.
- **A click by window pixels.** The driver captures no window screenshot here: it places a window only on compositors
  it knows, so `get_window_state` returns its `screenshot_error` and no image, and a click by window pixels is refused
  (`screenshot_context_missing`). Only when the driver's reason is that its latest read of the very window the call
  names had none (`no_screenshot`) does the admin click that pixel of the window; a window never read, read by another
  session, or another than the one named keeps the driver's refusal. Desktop pixels from `get_desktop_state` work
  through the driver itself.
- **Navigation and editing keys.** `press_key` and `hotkey` advertise `delete`, `insert`, `home`, `end`, `pageup` and
  `pagedown` (with `del`, `ins`, `pgup`/`page_up` and `pgdn`/`page_down`), and the driver checks and focuses the
  target first, then its injection client's whitelist refuses the key name (`cua-compositor does not support key ...`).
  The admin sends the display's `k` or `h` line for that process (or, into the focused application, for the browser
  shown alone); any other key name or earlier refusal stays the driver's.

Either click is the display's `c` line, which names the process the driver targets the window by, as `t` does: the
display clicks only while that process's window is the one shown, what is under the point is its own, and no pointer
grab, held button or drag of files would take the press elsewhere.

Where the agent acts (a click's point, an element's centre, for keys the last such place) goes to the session's
viewers as `AgentPointer`, at most ten a second each, and the viewer draws the agent's cursor there; no picture the
display composes, records or gives the agent holds it.

## The window list while a tab plays sound, or after its page crashed

The driver finds a listed window's process, its own number for it and its size by matching the window's title to its
accessible title, exactly. Chrome adds a tab's state to the accessible title alone: "Audio playing" while the tab shown
plays sound, "Crashed" once its page's process ended (both in the browser's language). The driver then lists the window
with no process, another number and no size, and its tools that name a window take neither: an agent that listed its
windows at such a moment could not act on them.

The admin fills the listing in (`admin/src/assist.rs`, `listed`). The display says, on its `w` line, the browser's
process (which the launcher tells it at each launch, `easi_files_v1` `browser_started`), how many toplevels are mapped,
and the number and size of the one shown. A window listed alone with its process is remembered with that toplevel;
listed without one while that toplevel still shows, it is given back with the process and number it had and the size
the display says. With nothing remembered of the toplevel shown (the first listing came in such a state), or with
several windows, a window gets the browser's process alone, which the keyboard tools go by. A display that does not
answer leaves the listing as the driver gave it.

## The DevTools finding

`inspect_page` needs none of this: it reads a page over the launcher's private pipe in every session. `run_javascript`,
which an agent gets only here, runs a script over that same pipe, in the page's own world. The driver's `browser_*` tools (exact tab binding,
navigation, DOM refs), which no agent is given, need Chrome's DevTools endpoint and prove its
ownership by reading `/proc/<chromium>/fd`. Linux allows that only to the same uid, or to `CAP_SYS_PTRACE` **and**
`CAP_DAC_READ_SEARCH` (the directory is mode 0500). A Cua Driver in its own uid E, as the design ([`design.md`](design.md) §3.1) first
proposed, is refused (`browser_binding_ambiguous`); `CAP_SYS_PTRACE` alone was measured insufficient, and
`CAP_DAC_READ_SEARCH` is the capability behind the `open_by_handle_at` container escape, so it is not granted.

The resulting design choice: DevTools is a per-session choice, off by default.

- **Without DevTools (default).** The design's arrangement: the driver runs as uid E with no capability and no DevTools
  port exists. Measured working: `list_windows`, `get_window_state` (AT-SPI tree), `click` by element and at desktop
  coordinates, `type_text` and `press_key`/`hotkey` through `cua-inject`, desktop-scope `scroll`, and
  `get_desktop_state` (measured in kiosk mode). Chrome now shows its tab strip and omnibox, so the agent navigates as
  a person does (`Ctrl+L`, typing, `Enter`).
- **With DevTools.** Chrome listens on its container's loopback only. The driver runs in a separate `agent`
  container with Chrome's uid and no capability, which Chrome's uid cannot reach into; [security](security.md#boundaries)
  (Agent) and [topology](topology.md) say how it is kept apart. Measured working: `browser_prepare` (`listening_socket_pid` ownership proof),
  `binding_quality: exact`, `browser_type`, `browser_click`, `browser_navigate` to an HTTPS page, plus everything of
  the default mode.
  A page read (`get_browser_state` with `snapshot_format: "semantic_v2"`) selects 300 elements in the driver's
  release, by priority (what is focused, in the viewport, near it, off screen), and its continuation returns the next
  ones by that priority, not by their place in the page. The driver built here
  (`scripts/patches/cua-driver-node-budget.patch`) takes `node_budget` (1 to 50,000) with the read: at or above the
  page's `total_nodes`, every element the read collected comes in one read, in the outline's order, unless they would
  take more than about 2 MiB of answer, when the rest still comes by continuation (`snapshot.omitted.budget` counts
  it). `snapshot.complete` says whether the read collected the whole page, which the budget does not change. Without
  the argument a read is as the release's. `agents::a_page_read_takes_as_many_elements_as_it_asks_for` and
  `agents::a_page_too_large_for_one_answer_is_read_in_pieces` check these against the running stack.
  The driver's release removed its `page` tool's script action (0.47.20), which `run_javascript` ran on. A script
  now goes the way `inspect_page`'s question does: from the admin to the display to the browser's launcher, which
  evaluates it in the page shown, in the page's own world, over its private DevTools pipe, and only in a session with
  Developer tools (`browser/launch/src/cdp.rs`, `Pages::run_script`). Its answer reads as the driver's did
  (`cdp.runtime.evaluate.user_gesture: ...`). With several tabs it runs on the page of the one that shows, and one
  that waits for ever (a promise nothing settles) is given up within about forty seconds, its call answering so and
  the session going on. One that runs on without yielding (a loop that does not end) keeps its page from answering
  anyone while it runs, a person who took control meanwhile too: the browser ends that evaluation when its thirty
  seconds are over (`Runtime.evaluate`'s `timeout`, which ends that script's own run and nothing else the page
  executes), and the page answers again. What a script set going and left (a timer, a promise's continuation) is the
  page's from then on, and is not ended.
  `agents::developer_tools_let_an_agent_run_a_script_on_the_page`,
  `agents::a_script_runs_on_the_page_of_the_tab_that_shows`,
  `agents::a_script_that_never_ends_is_given_up_and_the_session_goes_on` and
  `agents::a_script_that_never_yields_is_ended_in_its_page_when_its_time_is_over` check these.
