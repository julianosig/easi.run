//! The xdg shell: the browser's toplevels, each maximized at the output's origin and stacked, the newest (or the one
//! raised) on top, activated and focused; their popups composited above them. Only the top one shows, as on a desktop
//! where every window is maximized.

use smithay::desktop::{
    PopupKeyboardGrab, PopupKind, PopupManager, PopupUngrabStrategy, find_popup_root_surface, get_popup_toplevel_coords,
};
use smithay::reexports::wayland_protocols::xdg::shell::server::{xdg_positioner, xdg_toplevel};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::Serial;
use smithay::wayland::compositor::{get_role, with_states};
use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, SurfaceCachedState, ToplevelSurface, XDG_POPUP_ROLE, XDG_TOPLEVEL_ROLE,
    XdgPopupSurfaceData,
};
use wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1;
use wayland_server::Resource;

use crate::compose::{Rect, surface_tree};
use crate::limits;
use crate::state::{ClientKind, State, Toplevel, client_kind, toplevel_identity};

pub fn new_toplevel(state: &mut State, surface: ToplevelSurface) {
    let kind = state.dh.get_client(surface.wl_surface().id()).ok().and_then(|c| client_kind(&c));
    if kind != Some(ClientKind::Browser) {
        // Only the browser shows windows; the agent observes and injects but never draws.
        state.kill_client_of(surface.wl_surface(), "this client may not create windows");
        return;
    }
    let (width, height) = crate::state::output_size();
    surface.with_pending_state(|s| {
        s.states.set(xdg_toplevel::State::Maximized);
        s.size = Some((width as i32, height as i32).into());
    });
    let (title, app_id) = toplevel_identity(&surface);
    state.toplevels.push(Toplevel { surface, handle: None, mapped: false, wlr_handles: Vec::new(), title, app_id });
    activate_top(state);
}

/// Raises the toplevel at `index` to the top of the stack.
pub fn raise(state: &mut State, index: usize) {
    let top = state.toplevels.remove(index);
    state.toplevels.push(top);
    activate_top(state);
    state.schedule_frame();
}

/// The toplevel that is active: the top mapped one (the one shown), or the newest while none is mapped yet.
pub fn active_index(state: &State) -> usize {
    state.shown_index().unwrap_or(state.toplevels.len().saturating_sub(1))
}

/// A page's fullscreen (a video, F11) and back: fullscreen covers the whole output without Chrome's toolbar; leaving it
/// returns to maximized. The agent's toplevel lists hear of it at the next composition.
pub fn set_fullscreen(state: &mut State, surface: &ToplevelSurface, fullscreen: bool) {
    state.toplevel_states_stale = true;
    let (width, height) = crate::state::output_size();
    surface.with_pending_state(|s| {
        let (on, off) = (xdg_toplevel::State::Fullscreen, xdg_toplevel::State::Maximized);
        let (on, off) = if fullscreen { (on, off) } else { (off, on) };
        s.states.set(on);
        s.states.unset(off);
        s.size = Some((width as i32, height as i32).into());
    });
    if surface.is_initial_configure_sent() {
        surface.send_configure();
    }
    state.schedule_frame();
}

/// The active toplevel is activated and, when it is shown, focused, at once (with none shown, the keyboard is on none),
/// and the pointer's focus follows what it is over now; every other one is not, and another one's grabbing popups are
/// dismissed. The agent's toplevel lists hear of it at the next composition ([`announce_mapping`]).
fn activate_top(state: &mut State) {
    let last = active_index(state);
    for (index, top) in state.toplevels.iter().enumerate() {
        let active = index == last;
        let changed = top.surface.with_pending_state(|s| {
            let was = s.states.contains(xdg_toplevel::State::Activated);
            if active {
                s.states.set(xdg_toplevel::State::Activated);
            } else {
                s.states.unset(xdg_toplevel::State::Activated);
            }
            was != active
        });
        if changed && top.surface.is_initial_configure_sent() {
            top.surface.send_configure();
        }
    }
    state.toplevel_states_stale = true;
    state.schedule_frame();
    let Some(keyboard) = state.seat.get_keyboard() else { return };
    // None while no toplevel is shown: a hidden one takes no keys.
    let shown = state.browser_toplevel().map(|t| t.surface.wl_surface().clone());
    // Another toplevel's grabbing popups would keep the keyboard (their grab refuses every other focus): they go.
    if state.popup_grab.as_ref().is_some_and(|grab| grab.keyboard_grab_start_data().focus != shown) {
        end_popup_grab(state);
    }
    let serial = state.next_serial();
    keyboard.set_focus(state, shown, serial);
    crate::input::refresh_pointer_focus(state);
}

pub fn new_popup(state: &mut State, surface: PopupSurface, _positioner: PositionerState) {
    if !ancestry_in_bounds(&surface) {
        state.kill_client_of(surface.wl_surface(), POPUP_ANCESTRY);
        return;
    }
    unconstrain(state, &surface);
    let _ = state.popups.track_popup(PopupKind::Xdg(surface));
}

/// A popup's explicit grab (a menu): the keyboard goes to the topmost grabbing popup, and back to their toplevel once
/// the browser closes them, or a lease change or another toplevel shown dismisses them ([`end_popup_grab`]). The
/// pointer keeps its usual grabs: a popup grab would dismiss the popups on a press over another client's surface, and
/// every surface here is the browser's, while a press's click grab is what lets a drag start from a menu. Only the
/// toplevel shown may grab: a hidden one's popup is dismissed instead, so it never takes the keys meant for the one shown.
pub fn grab_popup(state: &mut State, surface: PopupSurface, serial: Serial) {
    let kind = PopupKind::Xdg(surface.clone());
    let Ok(root) = find_popup_root_surface(&kind) else { return };
    if state.browser_toplevel().map(|top| top.surface.wl_surface()) != Some(&root) {
        surface.send_popup_done();
        return;
    }
    let (seat, Some(keyboard)) = (state.seat.clone(), state.seat.get_keyboard()) else { return };
    let Ok(grab) = state.popups.grab_popup(root, kind, &seat, serial) else { return };
    keyboard.set_focus(state, grab.current_grab(), serial);
    keyboard.set_grab(state, PopupKeyboardGrab::new(&grab), serial);
    state.popup_grab = Some(grab);
}

/// A popup is gone: while the popup grab holds the keyboard, it goes at once to the topmost grabbing popup left, or
/// back to their toplevel when none is (the grab then lets it go and unsets itself).
pub fn follow_popup_grab(state: &mut State) {
    let (Some(grab), Some(keyboard)) = (state.popup_grab.clone(), state.seat.get_keyboard()) else { return };
    if keyboard.has_grab(grab.serial()) {
        let serial = state.next_serial();
        keyboard.set_focus(state, grab.current_grab().filter(|surface| surface.is_alive()), serial);
    }
}

/// Dismisses the popups that hold the keyboard, if any, and gives it back to their toplevel.
pub fn end_popup_grab(state: &mut State) {
    if let Some(grab) = &mut state.popup_grab {
        grab.ungrab(PopupUngrabStrategy::All);
        follow_popup_grab(state);
        state.popup_grab = None;
    }
}

const POPUP_ANCESTRY: &str = "a popup on a destroyed or unmade parent, or past the popup depth bound";

/// Whether every parent up a popup's chain (when it has one) is alive and already a popup or a toplevel, the root a
/// toplevel, and the chain at most [`limits::MAX_SURFACES`] popups long. smithay accepts a parent whose `wl_surface` is
/// gone (its `xdg_surface` keeps it, with its role and placement), a parent with no role yet, and a popup that is its
/// own parent, and walks the chain to its root to place the popup: without this check, a chain through destroyed
/// surfaces grows past every surface bound until its placement overflows `i32`, and a cycle never ends. Every parent
/// having its role (which never changes) when its child is made, a popup's chain is fixed from its creation on; it is
/// checked again at each later placement, since its parents may have gone since.
fn ancestry_in_bounds(popup: &PopupSurface) -> bool {
    let mut parent = popup.get_parent_surface();
    let mut popups = 0;
    while let Some(surface) = parent {
        if !surface.is_alive() {
            return false;
        }
        match get_role(&surface) {
            Some(XDG_POPUP_ROLE) => {}
            Some(XDG_TOPLEVEL_ROLE) => return true,
            _ => return false,
        }
        popups += 1;
        if popups > limits::MAX_SURFACES {
            return false;
        }
        parent = with_states(&surface, |states| {
            states.data_map.get::<XdgPopupSurfaceData>().and_then(|data| data.lock().unwrap().parent.clone())
        });
    }
    true
}

pub fn reposition(state: &mut State, surface: &PopupSurface, positioner: PositionerState) {
    if !ancestry_in_bounds(surface) {
        state.kill_client_of(surface.wl_surface(), POPUP_ANCESTRY);
        return;
    }
    surface.with_pending_state(|s| {
        s.geometry = positioner.get_geometry();
        s.positioner = positioner;
    });
    unconstrain(state, surface);
}

/// Whether the coordinates and sizes of an `xdg_positioner` request lie within [`limits::MAX_POSITIONER_EXTENT`].
pub fn positioner_in_bounds(request: &xdg_positioner::Request) -> bool {
    use xdg_positioner::Request;
    let values = match *request {
        Request::SetSize { width, height } => [width, height, 0, 0],
        Request::SetAnchorRect { x, y, width, height } => [x, y, width, height],
        Request::SetOffset { x, y } => [x, y, 0, 0],
        Request::SetParentSize { parent_width, parent_height } => [parent_width, parent_height, 0, 0],
        _ => [0; 4],
    };
    values.iter().all(|v| v.unsigned_abs() <= limits::MAX_POSITIONER_EXTENT.unsigned_abs())
}

/// Keeps a popup inside the output.
fn unconstrain(state: &State, popup: &PopupSurface) {
    let kind = PopupKind::Xdg(popup.clone());
    let Ok(root) = find_popup_root_surface(&kind) else { return };
    if !state.toplevels.iter().any(|t| t.surface.wl_surface() == &root) {
        return;
    }
    let whole = state.scene.whole();
    let mut target = smithay::utils::Rectangle::new((whole.x, whole.y).into(), (whole.w, whole.h).into());
    // The target is relative to the popup's parent; the toplevel's window geometry sits at the output origin.
    target.loc -= get_popup_toplevel_coords(&kind);
    popup.with_pending_state(|s| {
        s.geometry = s.positioner.get_unconstrained_geometry(target);
    });
}

fn window_geometry(surface: &WlSurface) -> smithay::utils::Rectangle<i32, smithay::utils::Logical> {
    let (width, height) = crate::state::output_size();
    with_states(surface, |states| {
        states
            .cached_state
            .get::<SurfaceCachedState>()
            .current()
            .geometry
            .unwrap_or_else(|| smithay::utils::Rectangle::new((0, 0).into(), (width as i32, height as i32).into()))
    })
}

pub fn handle_commit(state: &mut State, surface: &WlSurface) {
    if let Some(index) = state.toplevels.iter().position(|t| t.surface.wl_surface() == surface) {
        let toplevel = &state.toplevels[index].surface;
        if !toplevel.is_initial_configure_sent() {
            toplevel.send_configure();
        }
        // Mapped or unmapped since the last composition: focus follows at once, the agent's lists at the next one.
        let mapped = state.scene.is_mapped(surface);
        if mapped != std::mem::replace(&mut state.toplevels[index].mapped, mapped) {
            activate_top(state);
        }
    }
    state.popups.commit(surface);
    if let Some(PopupKind::Xdg(popup)) = state.popups.find_popup(surface)
        && !popup.is_initial_configure_sent()
    {
        let _ = popup.send_configure();
    }
}

/// Brings the agent's toplevel lists in line with the windows mapped and the one active, once per composition: a window
/// mapped since the last gets its handles, and one unmapped (a window being closed) loses them, the one below taking
/// over until it maps again. A browser that maps and unmaps a window as fast as it commits makes the agent's connection
/// at most one new handle per list a frame, and the agent's clients are counted again for them.
pub fn announce_mapping(state: &mut State) {
    let mut changed = false;
    for index in 0..state.toplevels.len() {
        let mapped = state.scene.is_mapped(state.toplevels[index].surface.wl_surface());
        if mapped && state.toplevels[index].handle.is_none() {
            map_toplevel(state, index);
            changed = true;
        } else if !mapped && state.toplevels[index].handle.is_some() {
            let handle = state.toplevels[index].handle.take();
            let wlr_handles = std::mem::take(&mut state.toplevels[index].wlr_handles);
            close_handles(state, handle, &wlr_handles);
            changed = true;
        }
    }
    if changed || std::mem::take(&mut state.toplevel_states_stale) {
        crate::wlr_toplevels::states_changed(state, active_index(state));
    }
    if changed {
        crate::relay::bound_agent_objects(state);
    }
}

fn map_toplevel(state: &mut State, index: usize) {
    let (title, app_id) = toplevel_identity(&state.toplevels[index].surface);
    let handle = state.foreign_toplevels.new_toplevel::<State>(title.clone(), app_id.clone());
    handle.send_done();
    let top = &mut state.toplevels[index];
    top.handle = Some(handle);
    top.title = title;
    top.app_id = app_id;
    let surface = top.surface.wl_surface().clone();
    crate::wlr_toplevels::mapped(state, index);
    state.output.enter(&surface);
    // Each time it maps again too, which a client can repeat once per composition.
    tracing::debug!(title = %state.toplevels[index].title, "browser window mapped");
}

/// Tells the agent's toplevel lists a window is gone (unmapped or destroyed): the ext list's handle, which smithay's
/// list then forgets (it keeps every handle it made until told, and a window maps anew as often as its client
/// likes), and the wlr list's handles.
fn close_handles(
    state: &mut State,
    handle: Option<ForeignToplevelHandle>,
    wlr_handles: &[ZwlrForeignToplevelHandleV1],
) {
    if let Some(handle) = handle {
        state.foreign_toplevels.remove_toplevel(&handle);
    }
    crate::wlr_toplevels::closed(wlr_handles);
}

pub fn update_identity(state: &mut State, surface: &ToplevelSurface) {
    let (title, app_id) = toplevel_identity(surface);
    let Some(top) = state.toplevels.iter_mut().find(|t| &t.surface == surface) else { return };
    if let Some(handle) = &top.handle {
        if title != top.title {
            handle.send_title(&title);
        }
        if app_id != top.app_id {
            handle.send_app_id(&app_id);
        }
        handle.send_done();
    }
    top.title = title;
    top.app_id = app_id;
    crate::wlr_toplevels::identity_changed(top);
}

pub fn toplevel_destroyed(state: &mut State, surface: &ToplevelSurface) {
    if let Some(index) = state.toplevels.iter().position(|t| &t.surface == surface) {
        let top = state.toplevels.remove(index);
        close_handles(state, top.handle, &top.wlr_handles);
        state.scene.forget(top.surface.wl_surface());
        activate_top(state);
    }
    state.schedule_frame();
}

/// The visible surfaces, bottom to top, with their output rectangles: the top mapped toplevel (every one is maximized,
/// so it covers the others) and its popups.
pub fn scene(state: &State) -> Vec<(WlSurface, Rect)> {
    let mut out = Vec::new();
    if let Some(top) = state.browser_toplevel() {
        let root = top.surface.wl_surface();
        surface_tree(root, toplevel_origin(top).0, &mut out);
        // Smithay lists popups top to bottom (the newest, and a child before its parent); the scene goes bottom to top.
        let popups: Vec<_> = PopupManager::popups_for_surface(root).collect();
        for (popup, location) in popups.into_iter().rev() {
            let popup_geometry = popup.geometry();
            let origin =
                (location.x.saturating_sub(popup_geometry.loc.x), location.y.saturating_sub(popup_geometry.loc.y));
            surface_tree(popup.wl_surface(), origin, &mut out);
        }
    }
    out
}

/// What the display draws: the scene, and above it the icon of a drag the browser started, at the pointer. Input never
/// hits the icon: it goes by [`scene`].
pub fn drawn(state: &State) -> Vec<(WlSurface, Rect)> {
    let mut out = scene(state);
    if let (Some(icon), Some(pointer)) = (&state.drag_icon, state.seat.get_pointer()) {
        let at = pointer.current_location();
        let origin = ((at.x as i32).saturating_add(icon.offset.x), (at.y as i32).saturating_add(icon.offset.y));
        surface_tree(&icon.surface, origin, &mut out);
    }
    out
}

/// The toplevel's root surface position in output pixels and its window geometry offset.
pub fn toplevel_origin(top: &Toplevel) -> ((i32, i32), (i32, i32)) {
    let geometry = window_geometry(top.surface.wl_surface());
    ((geometry.loc.x.saturating_neg(), geometry.loc.y.saturating_neg()), (geometry.loc.x, geometry.loc.y))
}
