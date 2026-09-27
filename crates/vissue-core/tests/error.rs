//! The typed error surface.
//!
//! Callers match on the variant; the text is for a person reading a terminal.
//! Both are checked here, because a caller that has to parse the text is the
//! thing the enum exists to prevent.

#![allow(missing_docs)]

use std::error::Error as _;

use vissue_core::error::Error;

#[test]
fn each_variant_says_what_went_wrong() {
    assert_eq!(
        Error::IssueNotFound {
            id: "atlas-1a2b".into()
        }
        .to_string(),
        "issue atlas-1a2b not found"
    );

    assert_eq!(
        Error::ClaimConflict {
            id: "atlas-1a2b".into(),
            holder: "worker-1".into(),
            claimed_at: Some("[2026-01-03 Sat]".into()),
        }
        .to_string(),
        "atlas-1a2b is claimed by worker-1 since [2026-01-03 Sat]; pass --force to take it over"
    );

    assert_eq!(
        Error::InvalidState {
            id: "atlas-4g5h".into(),
            state: "DONE".into()
        }
        .to_string(),
        "atlas-4g5h is already DONE; cannot claim"
    );

    assert_eq!(
        Error::StaleWrite {
            id: "atlas-1a2b".into(),
            expected_state: Some("STARTED".into()),
            actual_state: "CANCELLED".into(),
            expected_gen: None,
            actual_gen: None,
        }
        .to_string(),
        "atlas-1a2b is CANCELLED, not STARTED; write refused"
    );
}

#[test]
fn a_claim_with_no_recorded_time_still_reads() {
    let text = Error::ClaimConflict {
        id: "atlas-1a2b".into(),
        holder: "worker-1".into(),
        claimed_at: None,
    }
    .to_string();
    assert!(text.contains("an unknown time"), "{text}");
}

#[test]
fn a_self_block_reads_differently_from_a_cycle() {
    // Same variant, two sentences: telling someone an issue "cannot block
    // itself" is more use than telling them it would create a cycle.
    let itself = Error::BlockerCycle {
        blocker: "atlas-1a2b".into(),
        issue: "atlas-1a2b".into(),
    }
    .to_string();
    assert_eq!(itself, "issue atlas-1a2b cannot block itself");

    let cycle = Error::BlockerCycle {
        blocker: "atlas-1a2b".into(),
        issue: "atlas-3e4f".into(),
    }
    .to_string();
    assert_eq!(
        cycle,
        "adding atlas-1a2b -> atlas-3e4f would create a blocker cycle"
    );
}

#[test]
fn an_opaque_failure_shows_its_own_text_and_keeps_its_source() {
    let err = Error::Other(anyhow::anyhow!("read /tmp/x: permission denied"));
    assert_eq!(err.to_string(), "read /tmp/x: permission denied");
    assert!(err.source().is_some(), "Other carries the cause");
}

#[test]
fn only_the_opaque_variant_has_a_source() {
    let typed = Error::IssueNotFound { id: "x".into() };
    assert!(typed.source().is_none());
}

#[test]
fn a_typed_error_survives_a_trip_through_anyhow() {
    // A caller that wrapped the enum in anyhow must get the variant back
    // rather than an opaque wrapper around it.
    let original = anyhow::Error::new(Error::ClaimConflict {
        id: "atlas-1a2b".into(),
        holder: "worker-1".into(),
        claimed_at: None,
    });
    let recovered: Error = original.into();
    assert!(matches!(
        recovered,
        Error::ClaimConflict { ref holder, .. } if holder == "worker-1"
    ));
}

#[test]
fn an_unrelated_anyhow_error_becomes_the_opaque_variant() {
    let recovered: Error = anyhow::anyhow!("something else entirely").into();
    assert!(matches!(recovered, Error::Other(_)));
    assert_eq!(recovered.to_string(), "something else entirely");
}

#[test]
fn an_io_error_converts_and_keeps_its_message() {
    let io = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
    let converted: Error = io.into();
    assert!(matches!(converted, Error::Other(_)));
    assert!(converted.to_string().contains("no such file"));
}

#[test]
fn the_debug_shape_names_the_variant() {
    let text = format!(
        "{:?}",
        Error::InvalidState {
            id: "atlas-4g5h".into(),
            state: "CANCELLED".into()
        }
    );
    assert!(text.contains("InvalidState"), "{text}");
}

#[test]
fn a_chain_line_says_a_wrapped_message_once() {
    let wrapped: Error = anyhow::anyhow!("acme is not created here").into();
    let e = anyhow::Error::from(wrapped);
    assert_eq!(
        vissue_core::error::chain_line(&e),
        "acme is not created here"
    );
    let caused = anyhow::anyhow!("disk full").context("write refused");
    assert_eq!(
        vissue_core::error::chain_line(&caused),
        "write refused: disk full"
    );
}
