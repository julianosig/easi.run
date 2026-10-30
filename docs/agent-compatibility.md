# Agent compatibility

Measured 2026-10-18 on one x86-64 host (Linux 6.12, Docker 29.9, Yama `ptrace_scope=1`) with the pinned Cua Driver
`cua-driver-rs` 0.36.0 (release archive SHA-256 `05dae2160c86bdf4855c279d30898213d7a150edb1992b7470afdd69f0df4064`),
Debian Chromium 155.0.8059.79 (Wayland, kiosk, software rendering), and this repository's compositor. The probes ran
in throwaway containers; the hardened topology is proven again by the security tests (`tests/tests/security.rs`).
The browser image has since moved to Google Chrome 156 in an ordinary window (kiosk only with `BROWSER_KIOSK=1`),
which the agents end-to-end tests (`tests/tests/agents.rs`) drive through the driver's desktop tools.

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

Three things the pinned driver cannot do under this compositor, the admin does for it (`admin/src/assist.rs`), each on
the display's injection link: one `cua-inject v1` line on a stream connection of its own (`Link::Inject`), served as
the agent's own socket serves it, refused while a person holds control, and sent only within the agent's call, which
holds the agent's link, so a takeover waits for it.

- **Text beyond ASCII.** The driver's injection client types only printable ASCII, tab and newline (`cua-compositor
  cannot type '—'`). A window-targeted `type_text` of other text goes to the driver with no text, so the driver
  targets it as any (the element focused, or the point clicked, the window's binding checked; the display takes a `t`
  of no text), and the display then types the text with its `u` line, the way it types a person's: a character the
  keymap lacks with the spare key. The `u` line names the same target the driver's `t` does (the process, which the
  display holds to the window the driver's `t` bound it to, refusing one that replaced it), so a window mapped in between gets none of it;
  a call that names no process, nor an element of a snapshot read of one, is left to the driver. The agent gets the driver's answer, counting the characters typed. At most a person's
  text message (1,024 bytes) per call: a longer text is refused (`text_too_long`) before the driver is asked. Text in `run_steps` and `run_script` still reaches the driver's own refusal.
- **A click by element that has no action here.** For an element with no press (a menu item that only selects), the
  driver resolves the element, then falls back to an X11 pointer, which the agent's container has none of (`X11
  connection failed`). The admin clicks the centre of the element's frame in the last snapshot it relayed (read with
  `tree_format` `elements` or `both`: the default Markdown carries no frames), and answers as the driver's pixel click.
- **A click by window pixels.** The driver captures no window screenshot here: it places a window only on compositors
  it knows, so `get_window_state` returns its `screenshot_error` and no image, and a click by window pixels is refused
  (`screenshot_context_missing`). Only when the driver's reason is that its latest read of the very window the call
  names had none (`no_screenshot`) does the admin click that pixel of the window; a window never read, read by another
  session, or another than the one named keeps the driver's refusal. Desktop pixels from `get_desktop_state` work
  through the driver itself. (The admin strips screenshots only after a person first took control, as the redaction
  contract says, and that never empties the driver's own frame.)

Either click is the display's `c` line, which names the process the driver targets the window by, as `t` does: the
display clicks only while that process's window is the one shown, what is under the point is its own, and no pointer
grab, held button or drag of files would take the press elsewhere.

Where the agent acts (a click's point, an element's centre, for keys the last such place) goes to the session's
viewers as `AgentPointer`, at most ten a second each, and the viewer draws the agent's cursor there; no picture the
display composes, records or gives the agent holds it.

## The DevTools finding

The `browser_*` tools (exact tab binding, navigation, DOM refs) need Chrome's DevTools endpoint and prove its
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
