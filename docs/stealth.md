# What sites see

The browser is meant to read, to any site, as an ordinary desktop Google Chrome, without spoofing a single value from
JavaScript: everything a site measures is what this Chrome really is. This page lists what that takes and how public
bot and fingerprint detectors judge it.

## What makes it ordinary

- **Google Chrome** (stable, from Google's signed package), in a normal maximized window with its tab strip and
  omnibox, its own sandbox on, no automation flag (`navigator.webdriver` false; no DevTools TCP endpoint unless the
  session asks for one, only the launcher's private DevTools pipe for download sizes).
- **WebGL** on Mesa's llvmpipe through ANGLE, a real (software) GPU, never SwiftShader.
- **Online** (without DevTools; a DevTools session reads offline): Chrome runs in a network namespace whose only link
  reaches nothing, so `navigator.onLine` is true and the network quality estimate is measured from real requests
  (`navigator.connection.rtt`), while every byte still leaves only through the session's proxy
  ([security](security.md#boundaries)).
- **Storage**: the profile reads as an ordinary disk, so `navigator.storage.estimate()` reports the usual quota.
- **Fonts**: colour emoji, and Chinese, Japanese, and Korean, besides the Latin families.
- **Place**: the time zone and the languages (pages, workers, `Accept-Language`, `Intl`) are what the operator states
  for where the traffic leaves: the deployment's settings, or those of the egress the person picks for the session.
  Location, once a page is allowed it, comes from Chrome's own service, which places the browser by the address its
  requests leave from ([security](security.md#boundaries)).
- **Permissions** read as in a fresh profile: notifications, location, and camera are asked for, and their prompts
  show and answer.

`security::pages_see_an_ordinary_chrome`, `security::a_site_sees_an_ordinary_quota_but_stores_only_a_bounded_amount`,
and `security::a_session_leaves_only_through_its_egress_and_says_it_is_there` check these against the running stack.

## Detectors (2026-10-21, from a datacenter address, no person at the controls)

| Detector | Result |
|---|---|
| BrowserScan bot detection | every check Normal, no bot detected |
| BrowserScan fingerprint | authenticity 100% |
| CreepJS | 0% headless, 0% stealth, 50% "like headless" (see below) |
| rebrowser bot detector | every check green |
| deviceandbrowserinfo.com | "You are human" |
| bot.sannysoft.com | every check passed |
| bot.incolumitas.com | every test OK, `connectionRTT` included, but the legacy `WEBDRIVER` one, which fails for every current Chrome (`'webdriver' in navigator`); behavioural score 0 without a person moving the pointer |
| iphey.com | Trustworthy |
| Cloudflare managed challenge (scrapingcourse.com) | passed without interaction; now and then, from this address, it asks for its checkbox first |
| Pixelscan | Chrome on Linux, no automated behaviour, but "proxy detected" and "masking detected" (see below) |
| 2captcha.com Turnstile demo | blocked by its firewall (see below) |

What remains, and why:

- **CreepJS "like headless"** counts sixteen hints, eight of them here. Most hold for any desktop Chrome on Linux
  (Android-only APIs such as Web Share, Contacts, Content Index, and `downlinkMax`, and a light colour scheme). Two
  come from a screen with no taskbar: Chrome on Wayland learns no work area (only ChromeOS's protocol carries one), and
  the maximized window covers the whole screen.
- **Pixelscan "proxy detected"** and **2captcha's block**: the cause is not shown. A hypothesis: both judge the
  address, which here is a datacenter's; through a residential or ISP egress (`ADMIN_EGRESS_FILE`) sites see that
  egress's address instead, which has not been measured yet.
- **Pixelscan "masking detected"** is decided on its server and not explained; the TLS and HTTP/2 it sees are Chrome's
  own (the proxy only tunnels them). A hypothesis, not measured: rendering in software gives canvas and WebGL hashes
  that are rare among the real machines it compares against.
