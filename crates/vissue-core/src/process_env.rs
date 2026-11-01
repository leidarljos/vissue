//! Process environment with a thread-local overlay for tests.
//!
//! Production reads `std::env`. Tests call [`override_var`] so they never
//! need `set_var` / `remove_var`, which are unsafe in edition 2024.

use std::cell::RefCell;
use std::collections::HashMap;
use std::env::VarError;

thread_local! {
    static OVERLAY: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

/// Read `key`, honouring a test overlay if one is set on this thread.
///
/// # Errors
///
/// Returns [`VarError::NotPresent`] when the overlay unsets `key` or when
/// the real process environment has no such variable.
pub fn var(key: &str) -> Result<String, VarError> {
    if let Some(over) = OVERLAY.with(|m| m.borrow().get(key).cloned()) {
        return over.ok_or(VarError::NotPresent);
    }
    std::env::var(key)
}

/// Pretend `key` is `value` (`None` means unset) on this thread.
///
/// Used by tests. Production code never calls this.
pub fn override_var(key: &str, value: Option<&str>) {
    OVERLAY.with(|m| {
        m.borrow_mut()
            .insert(key.to_string(), value.map(str::to_string));
    });
}

/// Drop the overlay for `key` so later reads use the real process environment.
pub fn clear_override(key: &str) {
    OVERLAY.with(|m| {
        m.borrow_mut().remove(key);
    });
}
