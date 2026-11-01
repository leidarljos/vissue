//! Guest ``xdg_activation_v1`` on iced's Wayland connection.
//!
//! The HUD must activate on the same ``wl_display`` as the iced surface.
//! A second connection cannot name that surface. This module attaches as a
//! guest via ``Backend::from_foreign_display`` and does not own the display
//! or the surface.

#![allow(unsafe_code)]

use std::ffi::c_void;

use iced::window::raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Extract iced's ``wl_display`` and ``wl_surface`` pointers.
pub fn wayland_ptrs(window: &dyn iced::window::Window) -> Option<(*mut c_void, *mut c_void)> {
    let display = window.display_handle().ok()?;
    let surface = window.window_handle().ok()?;
    match (display.as_raw(), surface.as_raw()) {
        (RawDisplayHandle::Wayland(d), RawWindowHandle::Wayland(w)) => {
            Some((d.display.as_ptr(), w.surface.as_ptr()))
        }
        _ => None,
    }
}

/// Activate *window* with *token*. False when not Wayland, token empty, or bind fails.
pub fn activate(window: &dyn iced::window::Window, token: &str) -> bool {
    let Some((display, surface)) = wayland_ptrs(window) else {
        return false;
    };
    activate_raw(display, surface, token)
}

/// Guest activate using raw Wayland pointers from iced/winit.
pub fn activate_raw(display: *mut c_void, surface: *mut c_void, token: &str) -> bool {
    if display.is_null() || surface.is_null() {
        return false;
    }
    let Some(token) = crate::summon::sanitize_token(token) else {
        return false;
    };
    activate_guest(display, surface, &token)
}

/// Drop `XDG_ACTIVATION_TOKEN` and `DESKTOP_STARTUP_ID` from the process env.
///
/// Compositor tokens are single-use. Hide still unsets so a later spawn
/// cannot reuse them.
pub(crate) fn unset_activation_vars() {
    // SAFETY: compositor tokens are single-use. Edition 2024 marks
    // `remove_var` unsafe; the HUD process must drop both names.
    unsafe {
        std::env::remove_var("XDG_ACTIVATION_TOKEN");
        std::env::remove_var("DESKTOP_STARTUP_ID");
    }
}

/// Set compositor activation env vars. Tests must hold [`crate::env_lock`].
#[cfg(test)]
pub(crate) fn set_activation_vars(token: &str, startup_id: &str) {
    // SAFETY: test-only process env; callers hold `env_lock`.
    unsafe {
        std::env::set_var("XDG_ACTIVATION_TOKEN", token);
        std::env::set_var("DESKTOP_STARTUP_ID", startup_id);
    }
}

#[cfg(target_os = "linux")]
fn activate_guest(display: *mut c_void, surface: *mut c_void, token: &str) -> bool {
    use wayland_client::backend::{Backend, ObjectId};
    use wayland_client::protocol::wl_surface::WlSurface;
    use wayland_client::{Connection, Proxy};

    // SAFETY: iced/winit owns the display for the window lifetime. Guest mode
    // does not disconnect on drop.
    let backend = unsafe { Backend::from_foreign_display(display.cast()) };
    let conn = Connection::from_backend(backend);
    let mut queue = conn.new_event_queue::<Guest>();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut guest = Guest { activation: None };
    if queue.roundtrip(&mut guest).is_err() {
        return false;
    }
    let Some(activation) = guest.activation else {
        return false;
    };
    // SAFETY: iced owns the surface for the window lifetime. from_ptr does
    // not take ownership; drop must not destroy the foreign proxy.
    let Ok(id) = (unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }) else {
        return false;
    };
    let Ok(surf) = WlSurface::from_id(&conn, id) else {
        return false;
    };
    activation.activate(token.to_string(), &surf);
    activation.destroy();
    conn.flush().is_ok()
}

#[cfg(not(target_os = "linux"))]
fn activate_guest(_display: *mut c_void, _surface: *mut c_void, _token: &str) -> bool {
    false
}

#[cfg(target_os = "linux")]
struct Guest {
    activation:
        Option<wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1>,
}

#[cfg(target_os = "linux")]
impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, ()> for Guest {
    fn event(
        state: &mut Self,
        registry: &wayland_client::protocol::wl_registry::WlRegistry,
        event: wayland_client::protocol::wl_registry::Event,
        _: &(),
        _: &wayland_client::Connection,
        qh: &wayland_client::QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == "xdg_activation_v1"
        {
            state.activation = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

#[cfg(target_os = "linux")]
impl
    wayland_client::Dispatch<
        wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1,
        (),
    > for Guest
{
    fn event(
        _: &mut Self,
        _: &wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1,
        _: wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activate_raw_rejects_null_or_empty() {
        let null = std::ptr::null_mut();
        assert!(!activate_raw(null, null, "tok"));
        assert!(!activate_raw(null, null, ""));
        assert!(!activate_raw(null, null, "a\nb"));
    }
}
