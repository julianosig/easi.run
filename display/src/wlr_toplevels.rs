//! `zwlr_foreign_toplevel_manager_v1` for the agent: the browser's toplevel with its title, app id, and state. The
//! agent may activate it; it may not close, resize, or minimize it. Only clients of the privileged socket can see the
//! global.

use smithay::wayland::{Dispatch2, GlobalDispatch2};
use wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use wayland_server::{Client, DataInit, DisplayHandle, New, Resource, Weak};

use crate::state::{ClientKind, State, Toplevel, client_kind};

pub const VERSION: u32 = 3;

pub struct ManagerGlobal;

/// Which toplevel a handle describes, by its root surface.
pub struct HandleData {
    surface: Weak<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
}

impl GlobalDispatch2<ZwlrForeignToplevelManagerV1, State> for ManagerGlobal {
    fn bind(
        &self,
        state: &mut State,
        handle: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, ManagerGlobal);
        for index in 0..state.toplevels.len() {
            if state.toplevels[index].handle.is_some() {
                let active = index == crate::shell::active_index(state);
                announce(handle, client, &manager, &mut state.toplevels[index], active);
            }
        }
        state.wlr_managers.push(manager);
    }

    fn can_view(&self, client: &Client) -> bool {
        client_kind(client) == Some(ClientKind::Agent)
    }
}

impl Dispatch2<ZwlrForeignToplevelManagerV1, State> for ManagerGlobal {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        manager: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Request::Stop = request {
            state.wlr_managers.retain(|m| m != manager);
            manager.finished();
        }
    }

    fn destroyed(
        &self,
        state: &mut State,
        _client: wayland_server::backend::ClientId,
        manager: &ZwlrForeignToplevelManagerV1,
    ) {
        state.wlr_managers.retain(|m| m != manager);
    }
}

impl Dispatch2<ZwlrForeignToplevelHandleV1, State> for HandleData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _handle: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        if let zwlr_foreign_toplevel_handle_v1::Request::Activate { .. } = request
            && crate::input::agent_may_act(state)
            && let Ok(surface) = self.surface.upgrade()
            && let Some(index) = state.toplevels.iter().position(|t| t.surface.wl_surface() == &surface)
        {
            crate::shell::raise(state, index);
        }
    }

    fn destroyed(
        &self,
        state: &mut State,
        _client: wayland_server::backend::ClientId,
        handle: &ZwlrForeignToplevelHandleV1,
    ) {
        for top in &mut state.toplevels {
            top.wlr_handles.retain(|h| h != handle);
        }
    }
}

/// Every toplevel is maximized; the top one is also activated.
fn send_state(handle: &ZwlrForeignToplevelHandleV1, active: bool) {
    use zwlr_foreign_toplevel_handle_v1::State;
    let states: &[State] = if active { &[State::Activated, State::Maximized] } else { &[State::Maximized] };
    let states: Vec<u8> = states.iter().flat_map(|s| (*s as u32).to_ne_bytes()).collect();
    handle.state(states);
}

/// The stack changed: every handle gets its toplevel's state again; the one at `active` is activated.
pub fn states_changed(state: &State, active: usize) {
    for (index, top) in state.toplevels.iter().enumerate() {
        for handle in &top.wlr_handles {
            send_state(handle, index == active);
            handle.done();
        }
    }
}

fn announce(
    dh: &DisplayHandle,
    client: &Client,
    manager: &ZwlrForeignToplevelManagerV1,
    top: &mut Toplevel,
    active: bool,
) {
    let data = HandleData { surface: top.surface.wl_surface().downgrade() };
    let Ok(handle) = client.create_resource::<ZwlrForeignToplevelHandleV1, _, State>(dh, manager.version(), data)
    else {
        return;
    };
    manager.toplevel(&handle);
    handle.title(top.title.clone());
    handle.app_id(top.app_id.clone());
    send_state(&handle, active);
    handle.done();
    top.wlr_handles.push(handle);
}

/// Announces a newly mapped toplevel to every bound manager.
pub fn mapped(state: &mut State, index: usize) {
    let managers = state.wlr_managers.clone();
    for manager in managers {
        let Ok(client) = state.dh.get_client(manager.id()) else { continue };
        let dh = state.dh.clone();
        let active = index == crate::shell::active_index(state);
        announce(&dh, &client, &manager, &mut state.toplevels[index], active);
    }
}

pub fn identity_changed(top: &Toplevel) {
    for handle in &top.wlr_handles {
        handle.title(top.title.clone());
        handle.app_id(top.app_id.clone());
        handle.done();
    }
}

/// Tells the agent a window is gone: the end of its toplevel's handles.
pub fn closed(handles: &[ZwlrForeignToplevelHandleV1]) {
    for handle in handles {
        handle.closed();
    }
}
