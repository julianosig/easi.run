// What the lists of sessions show may have changed: the admin says so on one socket (admin/src/updates.rs), and the pages
// showing a list read it again then. The socket going away says so too: a change may have been missed, and reading the
// list is how a page finds its login over.

const AGAIN_AFTER_MS = 1000;
const AGAIN_AFTER_MAX_MS = 30_000;

const following = new Set<() => void>();
let socket: WebSocket | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;
let wait = AGAIN_AFTER_MS;

const tell = () => {
  for (const told of following) told();
};

function open(): void {
  const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
  const opened = new WebSocket(`${scheme}://${location.host}/ws/updates`);
  socket = opened;
  opened.onopen = () => {
    wait = AGAIN_AFTER_MS;
  };
  opened.onmessage = tell;
  opened.onclose = () => {
    if (socket !== opened) return;
    socket = null;
    tell();
    timer = setTimeout(open, wait);
    wait = Math.min(wait * 2, AGAIN_AFTER_MAX_MS);
  };
}

/** Calls `read` whenever the lists may have changed, until the function it returns is called. */
export function followLists(read: () => void): () => void {
  following.add(read);
  if (following.size === 1) open();
  return () => {
    following.delete(read);
    if (following.size > 0) return;
    clearTimeout(timer);
    const closing = socket;
    socket = null;
    closing?.close();
  };
}
