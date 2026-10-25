//! Resource bounds. What clients hold (pools, buffers, surfaces, pixel copies, protocol objects) is bounded per socket:
//! every client of the browser socket together, and every client of the agent socket together, so a browser or an
//! agent that opens more connections gets no more of the compositor. What a connection itself takes (descriptors, ids,
//! reads) is bounded per client. A client that crosses a bound is disconnected; the compositor stays up.

/// Live `wl_shm_pool` objects per socket.
pub const MAX_POOLS: usize = 32;
/// Total declared bytes of a socket's live pools.
pub const MAX_POOL_BYTES: usize = 64 * 1024 * 1024;
/// Live `wl_buffer` objects per socket.
pub const MAX_BUFFERS: usize = 128;
/// Live `wl_surface` objects per socket (subsurfaces, popups, and cursors included).
pub const MAX_SURFACES: usize = 64;
/// Live subsurfaces per socket.
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
/// Outputs' worth of pixels the compositor copies out of one socket's surfaces. Each surface's copy is at most the
/// output, but [`MAX_SURFACES`] of them would be half a gigabyte at 1920x1080. A browser's windows each take about one
/// output (those under the one shown keep theirs), its popups and cursors little, so only a browser past several
/// windows meets it.
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
/// Descriptors one `sendmsg` of a Wayland library carries; a client that sends more in one message is refused.
pub const MAX_MESSAGE_FDS: usize = 28;
/// Descriptors a client may have sent ahead of the requests that use them. Wayland libraries send descriptors ahead
/// of their messages; libwayland's own server buffers at most 1024 (its 4 KiB descriptor ring) and disconnects past it.
pub const MAX_UNUSED_FDS: usize = 1024;
/// Live protocol objects per socket, of every interface. Requests such as `wl_surface.frame`,
/// `wp_presentation.feedback`, or `wl_compositor.create_region` each create an object the library keeps until the
/// client commits or destroys it; this bounds them all. Chromium stays below 256 on every bench fixture.
pub const MAX_OBJECTS: usize = 4096;
/// Highest object id a client may use. The library keeps each client's objects in a vector indexed by id, so a client
/// that creates and destroys objects under ever higher ids would grow it without bound; libwayland clients reuse the
/// lowest free id, so their ids stay near their live count.
pub const MAX_CLIENT_ID: u32 = 16384;
/// `wl_keyboard` objects a client may create over its connection, released ones included. Each gets the seat's keymap
/// with a descriptor as it is created, and another at every switch of the keymap while it lives. Chrome creates one per
/// connection.
pub const MAX_KEYBOARDS: usize = 4;
/// Live clients of the browser socket: the launcher and Chrome's two connections (measured with Chrome 156), with room
/// for a relaunched Chrome connecting before the compositor has seen the last one hang up.
pub const MAX_BROWSER_CLIENTS: usize = 8;
/// Live clients of the agent socket.
pub const MAX_AGENT_CLIENTS: usize = 8;
/// Live connections of the agent's injection socket.
pub const MAX_INJECT_CONNECTIONS: usize = 8;
/// Characters one `t` line may type. The pinned Cua Driver waits 5 s for the answer to an injection line, then fails
/// the agent's tool call while the display goes on typing, so a retry would type the text twice. In release, with a
/// window whose client reads its keys, the display types some 60,000 characters a second: 128 KiB of lowercase letters
/// in 2.1 s, or 2.5 s with Shift on every one, half the driver's wait. A longer text is refused before any key.
pub const MAX_INJECT_TEXT: usize = 128 * 1024;
/// Bytes an injection connection may send ahead of what was served. The longest line is a `t` command with the longest
/// text it may type: the driver types only printable ASCII, tab and newline (a byte each), and it sends the text
/// hex-encoded, two digits a byte. So twice [`MAX_INJECT_TEXT`] in digits, with room for the command and its target.
pub const MAX_INJECT_LINE: usize = 2 * MAX_INJECT_TEXT + 4096;
/// Descriptors the library may hold for a client in events not sent yet: its output buffer holds 4 KiB, and the
/// smallest event that carries one (`wl_keyboard.keymap`) takes 16 bytes. The relay takes what it flushes to the pair
/// at its next turn.
const MAX_OUTGOING_FDS: usize = 4096 / 16;
/// Memory of events, and descriptors in them, the relay keeps for a client that has not accepted them yet. A client that
/// stops reading is disconnected past either. The bytes count the buffers the events are kept in, as Linux counts
/// against a socket's send buffer the memory of each message it holds, not only its bytes; they are about a quarter
/// more than the 416 KiB that Linux's two default send buffers (`net.core.wmem_default`, 208 KiB each) of the socket
/// pair and of the client's socket would hold together. The descriptors are keymaps, one for each keyboard at every switch of the keymap (see
/// `input::type_unicode`), which a client reading its events takes in milliseconds. Text typed into a window waits for
/// it well before either bound (see `input::UNREAD_LOW_WATER_BYTES`), so a browser that pauses is not disconnected.
pub const MAX_PENDING_EVENT_BYTES: usize = 512 * 1024;
pub const MAX_PENDING_EVENT_FDS: usize = 128;
/// The send buffer of a client's socket (`SO_SNDBUF`, which Linux doubles), so that what waits for a client waits in
/// the relay, where its descriptors are counted. Linux charges every message at least its own buffer of some 700
/// bytes, so the socket takes at most [`CLIENT_QUEUED_MESSAGES`] the client does not read (`relay::tests`).
pub const CLIENT_SEND_BUFFER: usize = 4096;
pub const CLIENT_QUEUED_MESSAGES: usize = 12;
/// Descriptors the compositor may hold at once, the soft limit it sets itself at startup (refusing to start where the
/// hard limit is lower), so the same limit holds wherever it runs: for every Wayland client its connection (the socket,
/// a second watch of it, and the pair to the library), the descriptors it may send ahead with one message more on its
/// way through the relay, and those of events on their way to it: in the library, in the relay (one message more than
/// its bound), and in its socket; for each socket its pools, live or kept by their buffers; for each injection
/// connection its socket and a watch; and 1024 for the compositor's own (listeners, the event loop, viewers, downloads,
/// uploads, pastes, recordings). Docker's default soft limit of 1024 would let one client's stray descriptors leave no
/// room for the next client.
pub const MAX_OPEN_FDS: usize = (MAX_BROWSER_CLIENTS + MAX_AGENT_CLIENTS)
    * (4 + MAX_UNUSED_FDS
        + 2 * MAX_MESSAGE_FDS
        + MAX_OUTGOING_FDS
        + MAX_PENDING_EVENT_FDS
        + MAX_MESSAGE_FDS
        + CLIENT_QUEUED_MESSAGES * MAX_MESSAGE_FDS)
    + 2 * (MAX_POOLS + MAX_BUFFERS)
    + 2 * MAX_INJECT_CONNECTIONS
    + 1024;
