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
