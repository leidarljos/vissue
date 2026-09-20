//! Float the HUD without a compositor include.
//!
//! iced `move_to` is a no-op on Wayland. When `SWAYSOCK` is set, the
//! board asks Sway to float and center the overlay `app_id`. Other
//! seats keep the icedtea overlay settings (undecorated, always-on-top,
//! X11 `override_redirect`).

use std::path::PathBuf;

use vissue_core::process_env;

/// Wayland `app_id` / X11 class the Sway criteria match.
pub const OVERLAY_APP_ID: &str = "me.rgoswami.vissue-hud";

/// One Sway output row used to pick the target display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputPick {
    /// Sway output name (`eDP-1`, `DP-1`, …).
    pub name: String,
    /// True when this output has the seat focus.
    pub focused: bool,
    /// True when the output is connected and enabled.
    pub active: bool,
}

/// Prefer focused, else first active. Skip inactive / nameless rows.
#[must_use]
pub fn pick_focused_or_active(outputs: &[OutputPick]) -> Option<usize> {
    let usable: Vec<usize> = outputs
        .iter()
        .enumerate()
        .filter(|(_, o)| o.active && !o.name.is_empty())
        .map(|(i, _)| i)
        .collect();
    if usable.is_empty() {
        return None;
    }
    usable
        .iter()
        .copied()
        .find(|&i| outputs[i].focused)
        .or(Some(usable[0]))
}

/// One Sway criteria command: float, move to output, center.
#[must_use]
pub fn place_command(output_name: &str) -> String {
    format!(
        "[app_id=\"{OVERLAY_APP_ID}\"] floating enable, \
         move to output \"{}\", move position center",
        escape_quotes(output_name)
    )
}

/// Whether a Sway IPC socket is configured.
#[must_use]
pub fn sway_available() -> bool {
    sway_sock().is_some()
}

/// Float and center the overlay. False when Sway is absent or the command
/// does not apply (window not mapped yet).
#[must_use]
pub fn place_overlay() -> bool {
    #[cfg(unix)]
    {
        let Some(sock) = sway_sock() else {
            return false;
        };
        let Some(outputs) = ipc_get_outputs(&sock) else {
            return false;
        };
        let Some(ix) = pick_focused_or_active(&outputs) else {
            return false;
        };
        ipc_run(&sock, &place_command(&outputs[ix].name))
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn sway_sock() -> Option<PathBuf> {
    let raw = process_env::var("SWAYSOCK").ok()?;
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    Some(PathBuf::from(t))
}

fn escape_quotes(name: &str) -> String {
    name.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(unix)]
mod ipc {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::time::Duration;

    use super::OutputPick;

    const MAGIC: &[u8] = b"i3-ipc";
    const RUN_COMMAND: u32 = 0;
    const GET_OUTPUTS: u32 = 3;

    pub(super) fn ipc_get_outputs(sock: &Path) -> Option<Vec<OutputPick>> {
        let raw = ipc_roundtrip(sock, GET_OUTPUTS, b"")?;
        parse_outputs(&raw)
    }

    pub(super) fn ipc_run(sock: &Path, cmd: &str) -> bool {
        ipc_roundtrip(sock, RUN_COMMAND, cmd.as_bytes()).is_some_and(|body| command_reply_ok(&body))
    }

    fn command_reply_ok(raw: &[u8]) -> bool {
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(raw) else {
            return false;
        };
        match v {
            serde_json::Value::Array(items) if !items.is_empty() => items.iter().all(item_success),
            serde_json::Value::Object(_) => item_success(&v),
            _ => false,
        }
    }

    fn item_success(item: &serde_json::Value) -> bool {
        item.get("success").and_then(serde_json::Value::as_bool) == Some(true)
    }

    fn ipc_roundtrip(sock: &Path, msg_type: u32, payload: &[u8]) -> Option<Vec<u8>> {
        let mut stream = UnixStream::connect(sock).ok()?;
        let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(400)));
        let mut header = Vec::with_capacity(MAGIC.len() + 8);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        header.extend_from_slice(&msg_type.to_le_bytes());
        stream.write_all(&header).ok()?;
        stream.write_all(payload).ok()?;
        stream.flush().ok()?;
        let mut rh = [0u8; 14];
        stream.read_exact(&mut rh).ok()?;
        if &rh[..6] != MAGIC {
            return None;
        }
        let len = u32::from_le_bytes(rh[6..10].try_into().ok()?) as usize;
        if len > 1_000_000 {
            return None;
        }
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).ok()?;
        Some(body)
    }

    pub(super) fn parse_outputs(raw: &[u8]) -> Option<Vec<OutputPick>> {
        let v: serde_json::Value = serde_json::from_slice(raw).ok()?;
        let arr = v.as_array()?;
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            out.push(OutputPick {
                name: item.get("name")?.as_str()?.to_string(),
                focused: item
                    .get("focused")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                active: item
                    .get("active")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            });
        }
        Some(out)
    }

    #[cfg(test)]
    pub(super) fn command_reply_ok_for_test(raw: &[u8]) -> bool {
        command_reply_ok(raw)
    }
}

#[cfg(unix)]
use ipc::{ipc_get_outputs, ipc_run};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_command_names_overlay_app_id() {
        let cmd = place_command("DP-1");
        assert!(cmd.contains(OVERLAY_APP_ID));
        assert!(cmd.contains("move to output \"DP-1\""));
        assert!(cmd.contains("move position center"));
        assert!(cmd.contains("floating enable"));
    }

    #[test]
    fn place_command_escapes_quotes() {
        let cmd = place_command(r#"DP"1"#);
        assert!(cmd.contains(r#"move to output "DP\"1""#));
    }

    #[test]
    fn pick_focused_wins_over_active() {
        let outs = [
            OutputPick {
                name: "eDP-1".into(),
                focused: false,
                active: true,
            },
            OutputPick {
                name: "DP-1".into(),
                focused: true,
                active: true,
            },
        ];
        assert_eq!(pick_focused_or_active(&outs), Some(1));
    }

    #[test]
    fn pick_first_active_when_none_focused() {
        let outs = [
            OutputPick {
                name: "eDP-1".into(),
                focused: false,
                active: true,
            },
            OutputPick {
                name: "DP-1".into(),
                focused: false,
                active: true,
            },
        ];
        assert_eq!(pick_focused_or_active(&outs), Some(0));
    }

    #[test]
    fn pick_skips_inactive_and_empty() {
        assert_eq!(pick_focused_or_active(&[]), None);
        let outs = [OutputPick {
            name: String::new(),
            focused: true,
            active: true,
        }];
        assert_eq!(pick_focused_or_active(&outs), None);
    }

    #[test]
    fn board_window_is_an_overlay() {
        let s = crate::app::board_window();
        assert!(!s.decorations);
        assert_eq!(s.level, iced::window::Level::AlwaysOnTop);
        let pop = crate::app::popout_window();
        assert!(pop.decorations, "the pop-out is a managed window");
        assert_eq!(pop.level, iced::window::Level::Normal);
        assert!(!s.exit_on_close_request);
        #[cfg(target_os = "linux")]
        {
            assert_eq!(s.platform_specific.application_id, OVERLAY_APP_ID);
            assert_ne!(
                pop.platform_specific.application_id, s.platform_specific.application_id,
                "a compositor rule on the overlay's id leaves the pop-out alone"
            );
            assert!(s.platform_specific.override_redirect);
        }
    }

    #[test]
    fn empty_swaysock_is_unavailable() {
        let _lock = crate::env_lock();
        process_env::override_var("SWAYSOCK", Some(""));
        assert!(!sway_available());
        process_env::override_var("SWAYSOCK", None);
        assert!(!sway_available());
        process_env::clear_override("SWAYSOCK");
    }

    #[cfg(unix)]
    #[test]
    fn parse_outputs_fixture() {
        let raw = br#"[
          {"name":"eDP-1","active":true,"focused":false},
          {"name":"DP-1","active":true,"focused":true}
        ]"#;
        let outs = ipc::parse_outputs(raw).unwrap();
        assert_eq!(outs.len(), 2);
        assert_eq!(pick_focused_or_active(&outs), Some(1));
    }

    #[cfg(unix)]
    #[test]
    fn parse_outputs_rejects_garbage() {
        assert!(ipc::parse_outputs(b"not-json").is_none());
        assert!(ipc::parse_outputs(b"{}").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn command_reply_requires_success() {
        assert!(ipc::command_reply_ok_for_test(br#"[{"success":true}]"#));
        assert!(ipc::command_reply_ok_for_test(
            br#"[{"success":true},{"success":true}]"#
        ));
        assert!(!ipc::command_reply_ok_for_test(br#"[{"success":false}]"#));
        assert!(!ipc::command_reply_ok_for_test(
            br#"[{"success":true},{"success":false}]"#
        ));
        assert!(!ipc::command_reply_ok_for_test(b"not-json"));
        assert!(!ipc::command_reply_ok_for_test(b"[]"));
    }
}
