# Third-party sources

Each directory is an unmodified release of a crate except for the patch listed here, committed beside it as
`NAME.patch`. `Cargo.toml` uses it through `[patch.crates-io]`; drop the patch once a release includes the fix.
`scripts/third_party.sh` downloads the release by its crates.io checksum, applies the patch, and fails on any other
difference; `scripts/check.sh` runs it whenever a file here changes.

## `wayland-backend` 0.3.17 (MIT)

From crates.io (`wayland-backend-0.3.17.crate`). One change, in the server's request handling: a `new_id` a client
sends must lie in the client's namespace (below `0xff000000`). Upstream accepts an id in the server's namespace and
inserts the object into the server-object store, so a client can grow that store without bound, for example with
`wl_display.sync` requests under ever higher server-namespace ids (the callbacks are destroyed at once and leave no
live object to count). The patch adds `ObjectMap::insert_at_client_id` in `src/rs/map.rs` and uses it at the four
places `src/rs/server_impl/client.rs` inserts a client-allocated object. `tests/tests/hostile.rs`
(`server_namespace_ids_disconnect`) proves it.

## `wtransport` 0.7.2 (MIT OR Apache-2.0)

From crates.io (`wtransport-0.7.2.crate`). One addition, in `src/endpoint.rs`:
`IncomingSessionFuture::with_quic_connection`, which takes a QUIC connection already established. Upstream takes only
the incoming connection or its handshake, and the driver task it starts keeps its own handle to the connection: a
caller that gives up on an attempt (the admin's deadline for a session's admission, `admin/src/udp.rs`) drops the
future, but the connection stays open for as long as the peer keeps it alive, without a ticket. With this the admin
holds its own handle, and closes the connection when the attempt is refused or runs out of time.
