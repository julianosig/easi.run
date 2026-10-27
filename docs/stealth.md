# What sites see

The browser is meant to read, to any site, as an ordinary desktop Google Chrome, without spoofing a single value from
JavaScript: everything a site measures is what this Chrome really is. This page lists what that takes and how public
bot and fingerprint detectors judge it.

## What makes it ordinary

- **Google Chrome** (stable, from Google's signed package), in a normal maximized window with its tab strip and
  omnibox, its own sandbox on, no automation flag (`navigator.webdriver` false; no DevTools TCP endpoint unless the
  session asks for one, only the launcher's private DevTools pipe, for download sizes and the pages a session opens).
- **WebGL** on Mesa's llvmpipe through ANGLE, a real (software) renderer, never SwiftShader.
- **Online** in every session without DevTools: Chrome runs in a network namespace whose only link reaches nothing,
  so `navigator.onLine` is true and the network quality estimate is measured from real requests
  (`navigator.connection.rtt`), while every byte still leaves only through the session's proxy
  ([security](security.md#boundaries)).
- **Storage**: the profile reads as an ordinary disk, so `navigator.storage.estimate()` reports the usual quota.
- **Fonts**: colour emoji, and Chinese, Japanese, and Korean, besides the Latin families.
- **Place**: the time zone and the languages (pages, workers, `Accept-Language`, `Intl`) are those stated
  for where the traffic leaves: the deployment's settings, or those of the egress the person picks for the session
  (what the administrator gave it, or what the country its check found suggests).
  Location, once a page is allowed it, comes from Chrome's own service, which places the browser by the address its
  requests leave from ([security](security.md#boundaries)).
- **Permissions** read as in a fresh profile: notifications, location, and camera read as asked for (`prompt`).

Against the running stack, `security::pages_see_an_ordinary_chrome` checks the automation flag, Google's brands, WebGL
through ANGLE and not SwiftShader, `navigator.onLine`, the fonts, the permissions, and the deployment's time zone and
languages; `security::a_site_sees_an_ordinary_quota_but_stores_only_a_bounded_amount` checks the storage quota; and
`security::an_egress_is_checked_before_use_and_a_session_leaves_only_through_it` checks an egress's place.

## Detectors (2026-10-21, from a datacenter address, no person at the controls)

| Detector | Result |
|---|---|
| BrowserScan bot detection | every check Normal, no bot detected |
| BrowserScan fingerprint | top authenticity score |
| CreepJS | 0% headless, 0% stealth |
| rebrowser bot detector | every check green |
| deviceandbrowserinfo.com | "You are human" |
| bot.sannysoft.com | every check passed |
| bot.incolumitas.com | `connectionRTT` OK |
| iphey.com | Trustworthy |
| Cloudflare managed challenge (scrapingcourse.com) | passed, mostly without interaction |
