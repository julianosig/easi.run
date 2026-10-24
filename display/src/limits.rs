//! Per-client resource bounds. A client that crosses one is disconnected; the compositor stays up.

/// Live `wl_shm_pool` objects per client.
pub const MAX_POOLS: usize = 32;
/// Total declared bytes of a client's live pools.
pub const MAX_POOL_BYTES: usize = 64 * 1024 * 1024;
/// Live `wl_buffer` objects per client.
pub const MAX_BUFFERS: usize = 128;
/// Live `wl_surface` objects per client (subsurfaces, popups, and cursors included).
pub const MAX_SURFACES: usize = 64;
/// Live subsurfaces per client.
pub const MAX_SUBSURFACES: usize = 32;
/// Commits a synchronized subsurface may make before its parent's commit applies them. The compositor keeps each one
/// until then; a browser commits a subsurface about once per commit of its parent.
pub const MAX_PENDING_SUBSURFACE_COMMITS: usize = 32;
/// Largest magnitude of a coordinate or size an `xdg_positioner` takes. smithay places a popup by adding its
/// positioner's values in plain `i32`, and a popup's parent by its own placement, up the popup's chain of parents.
/// smithay itself bounds that chain only by the client's object count (a parent's surface may be destroyed while its
/// popup role lives on, and a parent may get its role after its child), so the compositor checks it whenever it places
/// a popup: every parent alive and already a popup or the toplevel at the root, at most [`MAX_SURFACES`] popups up to
/// it (`shell::new_popup`, `shell::reposition`). Within both bounds no sum comes near `i32::MAX`.
/// Chromium's stay within the output.
pub const MAX_POSITIONER_EXTENT: i32 = 1 << 20;
/// Outputs' worth of pixels the compositor copies out of one client's surfaces. Each surface's copy is at most the
/// output, but [`MAX_SURFACES`] of them would be gigabytes at 4K. A browser's windows each take about one output (those
/// under the one shown keep theirs), its popups and cursors little, so only a client past several windows meets it.
pub const MAX_PIXEL_OUTPUTS: usize = 8;
/// Damage rectangles kept per surface between two frames, and pending on a surface until its commit; more collapse into
/// the whole surface.
pub const MAX_DAMAGE_RECTS: usize = 64;
/// Rectangles one `wl_region` may hold (Chromium's hold a few).
pub const MAX_REGION_RECTS: usize = 256;
/// Types one `wl_data_source` may offer, and their bytes in all (Chromium offers about ten, a few hundred bytes).
pub const MAX_OFFERED_TYPES: usize = 64;
pub const MAX_OFFERED_TYPE_BYTES: usize = 4096;
/// Extra bytes a buffer row may carry beyond `width * 4` for alignment.
pub const MAX_STRIDE_PADDING: u32 = 256;
/// Bytes of one client's requests dispatched per event-loop iteration, so no client can hold the loop.
pub const MAX_READ_PER_TURN: usize = 4096;
/// Descriptors a client may have sent ahead of the requests that use them. Wayland libraries send descriptors ahead
/// of their messages; libwayland's own server buffers at most 1024 (its 4 KiB descriptor ring) and disconnects past it.
pub const MAX_UNUSED_FDS: usize = 1024;
/// Live protocol objects per client, of every interface. Requests such as `wl_surface.frame`,
/// `wp_presentation.feedback`, or `wl_compositor.create_region` each create an object the library keeps until the
/// client commits or destroys it; this bounds them all. Chromium stays below 256 on every bench fixture.
pub const MAX_OBJECTS: usize = 4096;
/// Highest object id a client may use. The library keeps each client's objects in a vector indexed by id, so a client
/// that creates and destroys objects under ever higher ids would grow it without bound; libwayland clients reuse the
/// lowest free id, so their ids stay near their live count.
pub const MAX_CLIENT_ID: u32 = 16384;
/// Live clients of the browser socket (Chromium's browser and GPU processes need a few).
pub const MAX_BROWSER_CLIENTS: usize = 16;
/// Live clients of the agent socket.
pub const MAX_AGENT_CLIENTS: usize = 8;
/// Live connections of the agent's injection socket.
pub const MAX_INJECT_CONNECTIONS: usize = 8;
