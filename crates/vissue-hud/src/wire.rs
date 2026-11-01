//! Decode control-plane JSON for the palette. Types come from `vissue-control`.

use serde_json::Value;
use vissue_control::rpc::{IssueListResult, Notification, VaultChanged};

/// Parse an `issue/list` result body (snake_case rows, `claimed_by` on list).
///
/// # Errors
///
/// Returns an error if `value` is not an `issue/list` result body.
pub fn decode_issue_list(value: &Value) -> Result<IssueListResult, String> {
    serde_json::from_value(value.clone()).map_err(|err| err.to_string())
}

/// Parse a `vault/changed` notification, either the params object or a full
/// JSON-RPC envelope with `method` set.
///
/// # Errors
///
/// Returns an error if `value` is not a `vault/changed` params object or
/// envelope.
pub fn decode_vault_changed(value: &Value) -> Result<VaultChanged, String> {
    if value.get("method").and_then(Value::as_str) == Some("vault/changed") {
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        return match Notification::parse("vault/changed", params) {
            Notification::VaultChanged(body) => Ok(body),
            other => Err(format!("not vault/changed: {}", other.method())),
        };
    }
    serde_json::from_value(value.clone()).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn issue_list_fixture_keeps_claimed_by() {
        let page = decode_issue_list(&fixture("issue_list.json")).unwrap();
        assert_eq!(page.revision, 42);
        assert_eq!(page.generation, 3167);
        assert!(!page.unchanged);
        assert_eq!(page.matched, 1);
        assert_eq!(page.total, 6);
        assert_eq!(page.issues.len(), 1);
        let row = &page.issues[0];
        assert_eq!(row.id, "atlas-1a2b");
        assert_eq!(row.state, "STARTED");
        assert_eq!(row.priority, "A");
        assert_eq!(row.title, "Parse the manifest header");
        assert_eq!(row.project, "atlas");
        assert_eq!(row.claimed_by.as_deref(), Some("fixture-agent"));
        assert_eq!(row.claimed_at.as_deref(), Some("[2026-01-14 Wed 09:12]"));
        assert!(row.blocked_by.is_empty());
    }

    #[test]
    fn vault_changed_fixture_from_envelope() {
        let note = decode_vault_changed(&fixture("vault_changed.json")).unwrap();
        assert_eq!(note.generation, 3168);
        assert_eq!(note.revision, 43);
        assert_eq!(note.projects, ["atlas"]);
        assert_eq!(
            note.ids.as_deref(),
            Some([String::from("atlas-1a2b")].as_slice())
        );
    }

    #[test]
    fn vault_changed_accepts_params_object() {
        let params = serde_json::json!({
            "generation": 1,
            "revision": 2,
            "projects": ["beacon"]
        });
        let note = decode_vault_changed(&params).unwrap();
        assert_eq!(note.generation, 1);
        assert_eq!(note.revision, 2);
        assert_eq!(note.projects, ["beacon"]);
        assert!(note.ids.is_none());
    }
}
