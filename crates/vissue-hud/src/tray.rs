//! A StatusNotifier tray for the long-lived HUD. Left click toggles the
//! overlay through the summon socket, the menu offers the same and Quit,
//! and Quit leaves the HUD and nothing else: `vissue serve` stays up.
//! `VISSUE_HUD_TRAY=0` leaves the tray out. Linux only; elsewhere there is
//! no tray and the HUD runs as before.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// The tray's title.
pub const TITLE: &str = "vissue";
/// A themed icon name every Freedesktop theme carries.
pub const ICON: &str = "view-list-symbolic";
/// Menu labels, in order.
pub const MENU_TOGGLE: &str = "Show or hide the board";
/// The item that leaves the HUD.
pub const MENU_QUIT: &str = "Quit the HUD";

/// `VISSUE_HUD_TRAY=0` is the one way off.
#[must_use]
pub fn enabled() -> bool {
    match std::env::var("VISSUE_HUD_TRAY") {
        Ok(v) => v.trim() != "0",
        Err(_) => true,
    }
}

/// The menu as it reads, top to bottom.
#[must_use]
pub fn menu_labels() -> [&'static str; 2] {
    [MENU_TOGGLE, MENU_QUIT]
}

/// Start the tray on its own thread. The flag it returns is set by Quit;
/// the app reads it on its tick and exits. `None` when the tray is off.
#[must_use]
pub fn start() -> Option<Arc<AtomicBool>> {
    if !enabled() {
        return None;
    }
    let quit = Arc::new(AtomicBool::new(false));
    #[cfg(target_os = "linux")]
    linux::start(Arc::clone(&quit));
    Some(quit)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::summon::{self, SummonAction};

    struct VissueTray {
        quit: Arc<AtomicBool>,
    }

    fn toggle_board() {
        // The HUD's own socket: the request lands on the next tick like a
        // `vissue-hud toggle` from a shell.
        let _ = summon::send_command(SummonAction::Toggle);
    }

    impl ksni::Tray for VissueTray {
        fn id(&self) -> String {
            "vissue-hud".into()
        }

        fn title(&self) -> String {
            super::TITLE.into()
        }

        fn icon_name(&self) -> String {
            super::ICON.into()
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            toggle_board();
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::{MenuItem, StandardItem};
            vec![
                StandardItem {
                    label: super::MENU_TOGGLE.into(),
                    activate: Box::new(|_: &mut Self| toggle_board()),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                StandardItem {
                    label: super::MENU_QUIT.into(),
                    activate: Box::new(|this: &mut Self| this.quit.store(true, Ordering::Relaxed)),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    pub fn start(quit: Arc<AtomicBool>) {
        let _ = std::thread::Builder::new()
            .name("vissue-tray".into())
            .spawn(move || {
                use ksni::TrayMethods;
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                rt.block_on(async move {
                    // A bus without a StatusNotifier host refuses; the HUD
                    // runs without a tray, as it did.
                    if let Ok(_handle) = (VissueTray { quit }).spawn().await {
                        std::future::pending::<()>().await;
                    }
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_toggles_then_quits() {
        assert_eq!(menu_labels(), [MENU_TOGGLE, MENU_QUIT]);
        assert!(MENU_QUIT.contains("HUD"), "Quit names what it leaves");
    }
}
