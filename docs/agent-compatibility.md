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
