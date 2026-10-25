//! `zwp_virtual_keyboard_v1` for the agent. Keys go through the seat's own keymap, which the agent's matches (a US
//! layout), so the agent's keymap is never mapped, read, or compiled: a few hundred bytes of XKB can make libxkbcommon
//! allocate gigabytes (a key code near 2^28) or abort (a shift level near 2^31), and its size bounds neither. The
//! descriptor is checked the way a `wl_shm` pool's is (see [`crate::shm`]): a tmpfs file at least as large as the size
//! it declares, that size at most [`MAX_KEYMAP_BYTES`]. Only clients of the privileged socket can see the global.

use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};

use smithay::backend::input::{KeyState, Keycode};
use smithay::input::keyboard::{FilterResult, KeyboardSource};
use smithay::reexports::wayland_protocols_misc::zwp_virtual_keyboard_v1::server::{
    zwp_virtual_keyboard_manager_v1::{self, ZwpVirtualKeyboardManagerV1},
    zwp_virtual_keyboard_v1::{self, ZwpVirtualKeyboardV1},
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};
use wayland_server::protocol::wl_keyboard::KeymapFormat;
use wayland_server::{Client, DataInit, DisplayHandle, New, Resource, backend::ClientId};

use crate::shm;
use crate::state::{ClientKind, State, client_kind};

pub const VERSION: u32 = 1;

/// The largest keymap the compositor accepts, about four times a full evdev US keymap.
const MAX_KEYMAP_BYTES: usize = 256 * 1024;

/// User data of the `zwp_virtual_keyboard_manager_v1` global.
pub struct ManagerGlobal;

/// One virtual keyboard: the seat source its keys come from, and whether it sent a keymap.
pub struct Keyboard {
    source: KeyboardSource,
    has_keymap: AtomicBool,
}

impl GlobalDispatch2<ZwpVirtualKeyboardManagerV1, State> for ManagerGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwpVirtualKeyboardManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ManagerGlobal);
    }

    fn can_view(&self, client: &Client) -> bool {
        client_kind(client) == Some(ClientKind::Agent)
    }
}

impl Dispatch2<ZwpVirtualKeyboardManagerV1, State> for ManagerGlobal {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &ZwpVirtualKeyboardManagerV1,
        request: zwp_virtual_keyboard_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        if let zwp_virtual_keyboard_manager_v1::Request::CreateVirtualKeyboard { id, .. } = request {
            data_init
                .init(id, Keyboard { source: KeyboardSource::new_auxiliary(), has_keymap: AtomicBool::new(false) });
        }
    }
}

impl Dispatch2<ZwpVirtualKeyboardV1, State> for Keyboard {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        keyboard: &ZwpVirtualKeyboardV1,
        request: zwp_virtual_keyboard_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        use zwp_virtual_keyboard_v1::{Error, Request};
        match request {
            Request::Keymap { format, fd, size } => {
                let checked = match format {
                    f if f == KeymapFormat::NoKeymap as u32 => Ok(()),
                    f if f == KeymapFormat::XkbV1 as u32 => check_keymap(&fd, size),
                    _ => Err("unsupported keymap format"),
                };
                match checked {
                    Ok(()) => self.has_keymap.store(true, Ordering::Relaxed),
                    Err(why) => keyboard.post_error(Error::NoKeymap, why),
                }
            }
            Request::Key { time, key, state: pressed } => {
                if !self.has_keymap.load(Ordering::Relaxed) {
                    keyboard.post_error(Error::NoKeymap, "`key` sent before keymap");
                } else if key > u32::from(display_protocol::keys::MAX) {
                    keyboard.post_error(Error::NoKeymap, "key code beyond the evdev range");
                } else {
                    let pressed = if pressed == 0 { KeyState::Released } else { KeyState::Pressed };
                    agent_key(state, self.source, key, pressed, time);
                }
            }
            // Modifiers follow the keys the agent presses; a mask set directly is not applied.
            Request::Modifiers { .. } if !self.has_keymap.load(Ordering::Relaxed) => {
                keyboard.post_error(Error::NoKeymap, "`modifiers` sent before keymap");
            }
            _ => {}
        }
    }

    /// A keyboard the agent destroyed, or lost with its connection: what it still holds is released, and it is
    /// forgotten (the set stays as small as the live keyboards).
    fn destroyed(&self, state: &mut State, _client: ClientId, _resource: &ZwpVirtualKeyboardV1) {
        if let KeyboardSource::Auxiliary(id) = self.source
            && state.agent_keyboards.remove(&id)
            && let Some(keyboard) = state.seat.get_keyboard()
        {
            keyboard.release_source(state, self.source);
        }
    }
}

/// One of the agent's keys, through the seat while the agent may act.
fn agent_key(state: &mut State, source: KeyboardSource, key: u32, pressed: KeyState, time: u32) {
    let Some(keyboard) = state.seat.get_keyboard() else { return };
    if !crate::input::agent_may_act(state) {
        return;
    }
    if let KeyboardSource::Auxiliary(id) = source {
        state.agent_keyboards.insert(id);
    }
    let serial = state.next_serial();
    // Protocol key codes are evdev codes, 8 below xkb's.
    let time = smithay::backend::input::InputTime::from_millis(time);
    keyboard.input_from_source(source, state, Keycode::new(key + 8), pressed, serial, time, |_, _, _| {
        FilterResult::<()>::Forward
    });
}

/// Checks the descriptor of an `xkb_v1` keymap of `size` bytes, without mapping, reading, or compiling it.
fn check_keymap(fd: &OwnedFd, size: u32) -> Result<(), &'static str> {
    let size = size as usize;
    if size == 0 || size > MAX_KEYMAP_BYTES {
        return Err("keymap size out of bounds");
    }
    if !shm::tmpfs_backed(fd) || shm::regular_file_size(fd).is_none_or(|len| len < size as u64) {
        return Err("keymap must be a tmpfs file at least as large as declared");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memfd_with(bytes: &[u8], len: u64) -> OwnedFd {
        let fd = rustix::fs::memfd_create("keymap", rustix::fs::MemfdFlags::CLOEXEC).expect("memfd_create");
        rustix::io::pwrite(&fd, bytes, 0).expect("pwrite");
        rustix::fs::ftruncate(&fd, len).expect("ftruncate");
        fd
    }

    #[test]
    fn a_keymap_is_accepted_only_within_its_file_and_its_bound() {
        let mut keymap = crate::inject::CharTable::new().keymap.into_bytes();
        keymap.push(0);
        let len = keymap.len() as u64;
        assert!(keymap.len() * 3 < MAX_KEYMAP_BYTES, "the bound leaves room for a real keymap ({len} bytes)");
        assert_eq!(check_keymap(&memfd_with(&keymap, len), len as u32), Ok(()));
        // Declared past the file's end: the SIGBUS case of a mapping.
        assert!(check_keymap(&memfd_with(&keymap, len), len as u32 + 4096).is_err());
        assert!(check_keymap(&memfd_with(&keymap, 4096), 1 << 20).is_err());
        let past = MAX_KEYMAP_BYTES as u64 + 1;
        assert!(check_keymap(&memfd_with(&keymap, past), past as u32).is_err());
        assert!(check_keymap(&memfd_with(&keymap, len), 0).is_err());
        let (read, _write) = rustix::pipe::pipe().expect("a pipe");
        assert!(check_keymap(&read, 64).is_err());
    }
}
