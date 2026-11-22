# Agent compatibility

Measured 2026-10-18 on one x86-64 host (Linux 6.12, Docker 29.9, Yama `ptrace_scope=1`) with Cua Driver
`cua-driver-rs` 0.36.0, Debian Chromium 155.0.8059.79 (Wayland, kiosk, software rendering), and this repository's
compositor. The probes ran in throwaway containers; the hardened topology is proven again by the security tests
(`tests/tests/security.rs`). The browser image has since moved to Google Chrome in an ordinary window (kiosk only
with `BROWSER_KIOSK=1`), and the pinned driver to `cua-driver-rs` 0.56.9, built from its source (commit
`72f5415fe8c229aebc1bc647655c92fb9f5956e2`) with two changes, below: the probes were not run again with them, and the
agents end-to-end tests (`tests/tests/agents.rs`) drive that browser through that driver's desktop tools.

That driver is compiled once for each version, commit and set of patches (the root Dockerfile's `driver` target) and
published as `ghcr.io/julianosig/easi.run-driver`, with the patches and its recipe beside the binary (`/cua/`); the
agent image takes it from there by digest, its build stops unless the recipe is this repository's, and `--build-arg
CUA_DRIVER=driver` compiles it in place ([`CONTRIBUTING.md`](../CONTRIBUTING.md), Releases).

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
- **A click or scroll by window pixels, and desktop scroll.** The driver captures no window screenshot here: it places
  a window only on compositors it knows, so `get_window_state` returns its `screenshot_error` and no image, and a click
  or scroll by window pixels is refused (`screenshot_context_missing`). Only when the driver's reason is that its
  latest read of the very window the call names had none (`no_screenshot`) does the admin click or scroll at that pixel
  of the window (and at an element's centre when a scroll by element falls back to X11); a window never read, read by
  another session, or another than the one named keeps the driver's refusal. Desktop pixels from `get_desktop_state`
  work through the driver itself (its `d` click and `e` wheel lines on `cua-inject v1`).
- **Navigation and desktop keys.** `press_key` and `hotkey` advertise `delete`, `insert`, `home`, `end`, `pageup` and
  `pagedown` (with `del`, `ins`, `pgup`/`page_up` and `pgdn`/`page_down`), which the driver's injection client omits
  from its named-key whitelist after checking and focusing the target (`cua-compositor does not support key ...`); and
  into the focused application (`scope` `desktop` with no process) the driver asks for a foreground target that only
  `bring_to_front` (not offered here) sets (`foreground_unavailable: cua-compositor has no verified foreground target`).
  The admin sends the display's `k` or `h` line for that window's process, or, into the focused application, for the
  browser shown alone; any other key name or earlier refusal stays the driver's.

Either click or scroll is the display's `c` or `s` line, which names the process the driver targets the window by, as
`t` does: the display acts only while that process's window is the one shown, what is under the point is its own, and
no pointer grab, held button or drag of files would take the press or wheel elsewhere.

Where the agent acts (a click's point, an element's centre, for keys the last such place) goes to the session's
viewers as `AgentPointer`, at most ten a second each, and the viewer draws the agent's cursor there; no picture the
display composes, records or gives the agent holds it.

## The window list while a tab plays sound, shows an error page, or after its page crashed

The driver finds a listed window's process, its own number for it and its size by matching the window's title to its
accessible title, exactly. Chrome adds a tab's state to the accessible title alone: "Audio playing" while the tab shown
plays sound, "Network error" while it shows the browser's own error page (a host that does not answer, a site that is
down), "Crashed" once its page's process ended (all in the browser's language). The driver then lists the window
with no process, another number and no size, and its tools that name a window take neither: an agent that listed its
windows at such a moment could not act on them.

The admin sees to the listing (`admin/src/agent.rs` `relisted`, `admin/src/assist.rs`). When the driver placed none of
the windows of a listing of every window, the admin asks it again for the windows of the browser's process alone,
which the driver lists by their accessibility, with the numbers and sizes its tools take: that answer is given when
each window in it has its process and a size, and otherwise the listing is asked once more as it first was. Nothing
more is asked once a person took control. The admin then fills the listing in (`listed`). The display says, on its `w` line, the browser's
process (which the launcher tells it at each launch, `easi_files_v1` `browser_started`), how many toplevels are mapped,
and the number and size of the one shown. A window listed alone with its process is remembered with that toplevel;
listed without one while that toplevel still shows, it is given back with the process and number it had and the size
the display says. With nothing remembered of the toplevel shown, or with
several windows, a window gets the browser's process alone, which the keyboard tools go by. A display that does not
answer leaves the listing as the driver gave it.
`agents::a_window_first_listed_on_an_error_page_is_one_the_tools_take` and
`agents::a_window_keeps_its_process_in_the_agents_list_while_its_tab_plays_sound` check these.

The driver's own binding of a window to its tab (`get_browser_state`'s `binding_quality`, in a session with Developer
tools) goes by the same titles and is not the admin's to fill in: on an error page it stays `exact` while the browser
has that one window, and falls to `heuristic` (which allows no change to the page) once a second one opens beside it,
as Chrome's downloads bubble does when a navigation from that page turns into a download. A session that starts on an
empty page (`about:blank`, which `start_url` admits beside http(s) addresses) shows no error page to begin with, and
keeps its binding through such a download
(`agents::a_session_started_on_an_empty_page_keeps_its_window_bound_through_a_download`).

## Rich-text `contenteditable` editors

Two things come up when driving rich-text `<div contenteditable="true">` editors through the desktop tools:

- **Editors whose token is not an `entry`.** Draft.js and Lexical set `role="textbox"` on the editable root, which
  exposes an AT-SPI `entry` that `type_text` can focus by `element_token`. ProseMirror (`<div class="ProseMirror"
  contenteditable="true">` without `role="textbox"`) and inner blocks of a rich-text editor resolve to `paragraph` or
  `section`, and `type_text` by that token is refused with `AT-SPI Component.GrabFocus returned false`. Clicking inside
  the editor first and calling `type_text` with `pid` and `window_id` alone works.
- **Long `type_text` calls.** A window-targeted `type_text` has 4.5 s from when its line began for the window to read
  its keys, inside the driver's 5 s wait, and Chrome reads about 4,000 characters a second at `idle_fps=5`. A text of
  up to about 17,000 characters is typed whole (measured on a `<textarea>`, Draft.js and Lexical); a longer one is cut
  with `target-not-reading` when that time is up, and what was typed until then stays in the page, so longer texts go
  in parts. A window that reads nothing at all for 2 s is given up on at once. An answer of `ok` says the keys were
  sent, and that Chrome said it handled them or the time for it to say so ran out: a heavy editor can keep the page's
  main thread busy applying them for much longer before the whole text shows (45 to 90 s for 8,000 characters on the
  Lexical playground with its debug tree open).

## Screenshots

The driver answers `get_desktop_state` with a 32-bit PNG that it barely compresses: about 300 kB for a page at
1280x800. The admin answers the same picture as a PNG of 256 colours (`admin/src/shots.rs`), about a quarter of that
(11 pages: 2,930 kB as the driver made them, 802 kB as answered), for some 50 ms of one processor a screenshot, one
at a time. It is still a PNG of the display's own size, so a point read from it is the same point for the driver,
whose own capture the click is checked against.

It loses colour: the 256 are chosen for each picture, so two colours that are close (two shades of a status, the
bands of a heatmap, a faint marker) can come out as one. Text reads the same: both programs' models transcribed 64
codes of 64 from it, at 10 to 16 pixels and several contrasts, as from the driver's own. Nothing asks for the
driver's own picture instead (its `screenshot_out_file` writes one, to a file in the agent's own container that
nothing here reads back; one that could not be made smaller is answered as the driver made it): an agent that needs
what a colour is exactly has what the page says of it (a style in its markup, read
with `inspect_page`), which a canvas or an image does not say. A `capture_id` binds a click to the driver's capture,
which the answered picture is in everything but its colours.

What it saves is mostly not on the wire: Codex and Claude Code keep every tool answer in the conversation's file for
good, and screenshots were 11 of the 14.5 GB of one conversation of eleven days. A picture that is not a still PNG
of eight bits a channel in colour, that has anything see-through or says how its colours are to be read, or that
would not come out smaller, is answered as it came.

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
  A page read (`get_browser_state` with `snapshot_format: "semantic_v2"`) selects 300 elements, by priority (what
  is focused, in the viewport, near it, off screen), and says how many it left out (`snapshot.omitted.budget`). Its
  `snapshot.continuation` returns the next ones by that priority, each piece in the page's order; a caller that
  follows it at once, until none comes back, gets every element (a continuation works once, and another read of the
  tab or a navigation makes it stale). Measured on 56 public pages of more than 300 elements: every element reached,
  about 40 ms for each call after the first. `snapshot.complete` says whether the read collected the whole page.
  `agents::a_page_larger_than_one_read_comes_whole_by_its_continuations` checks this against the running stack.
  The first change made to the driver here (`scripts/patches/cua-driver-occlusion.patch`) is in how a read marks an
  element covered (`page_occluded`): the release tests every pair of an element in the viewport and a box of the
  page's layout, walking the DOM up from both before its cheap tests, which on a dense page (a social feed) takes
  seconds of one core for each read; the patch takes once the few boxes that can cover anything (painted, fixed or
  absolute, taking the pointer, shown, with an area) and walks the DOM only for one that covers the element. Each
  element's visibility is the same; only outline lines that share a document order, which have no set order in the
  release either, may come in another. Measured in [benchmark](benchmark.md#agent-page-reads).
  The second (`scripts/patches/cua-driver-active-tab.patch`) is in how binding a window (`get_browser_state` with
  `pid` and `window_id`) names its selected tab. The release proves it only by title: the one tab whose title the
  window's title holds. With several tabs of that title (tabs Chrome restores from a saved profile and never loads
  show their URL as their title, as a page still loading does), or for a moment after a navigation while the window's
  title still shows the last page's, it names no tab active, and an agent that reads the active tab loses its page
  (seen on x.com: one answer after a navigation, and a minute of answers with two restored tabs of the same title).
  The patch then asks Chrome's own tab strip, when the window itself is proven without its title (by its bounds, or as
  the browser's only window; not matched by title alone, nor by title among windows of the same bounds, either of which
  may be another window): the `tab` target its window has selected (`embedderData.tabActive`),
  and that tab's page, attached to as a read attaches to it, without running anything in a page or selecting a tab; a
  Chrome that does not say leaves the answer as the release's. A window title that, a moment behind, holds another
  tab's title alone still names that tab, as in the release.
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
