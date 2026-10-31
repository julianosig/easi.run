# Security policy

easi.run runs untrusted web pages next to a person's input, so security reports are welcome and taken
seriously. The threat model, the boundaries, and the tests that prove them are in
[`docs/security.md`](docs/security.md).

## Reporting a vulnerability

Report privately through GitHub's **Report a vulnerability** button on this repository's Security tab. Do not open a
public issue for a vulnerability.

A useful report includes:

- what an attacker controls (a page, a Wayland client, a viewer, an agent token, or the network);
- what they gain;
- the steps or a proof of concept;
- the commit you tested.

You will get an acknowledgement within a week. A fix comes with a test that proves it, and the report is credited
unless you ask otherwise.

## Scope

In scope is anything that crosses a boundary listed in [`docs/security.md`](docs/security.md). For example:

- escaping the browser container, or reaching the network other than through the proxy;
- reaching the compositor's privileged socket, or crashing the compositor from a Wayland client;
- injecting input without the lease;
- reaching Docker other than through the controller's closed set;
- defeating the admin application's authentication or CSRF protection;
- reaching, with an agent token, a session it neither started nor may reach through a saved session that allows it,
  or anything behind the administrator's login.

The hosted demo at <https://easi.run/demo/> is in scope too, including leaving its walled garden. Everyone shares it,
so test it as an ordinary visitor: one visit at a time, and no load testing, denial of service, or automated scanning.
For anything heavier, run your own installation.

## Supported versions

Only the latest commit on `main` receives fixes.
