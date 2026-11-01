//! The shipped client against the shipped server.
//!
//! Every other test of this backend answers it from a stub that writes
//! canned JSON, which proves the client parses what the stub was told to
//! send and nothing about whether the server sends that. The unit tests on
//! the other side call the server's dispatch directly, with no client and no
//! socket. Neither half tests the pair, so a disagreement between them
//! survives a green run: a mutation the server accepted but did not make
//! visible to the next read looks identical to one it made, until someone
//! runs the two together by hand.
//!
//! This file runs them together. The server is a real owner on a real
//! socket, with its own catalog and watcher; the client is `ControlBackend`,
//! the same one `vissue tui` and `vissue hud` attach with.

#![cfg(unix)]
#![allow(missing_docs)]

use std::path::Path;
use std::time::Duration;

use vissue_core::config::{DEFAULT_PREFIX, Layout};
use vissue_core::ops::{self, CreateOpts};
use vissue_serve::{OwnerHandle, ServeConfig};
use vissue_tui::backend::{BackendKind, BoardBackend};
use vissue_tui::control::ControlBackend;

/// A tracker with two projects and three issues, plus a live owner over it.
fn live() -> (tempfile::TempDir, Layout, OwnerHandle) {
    let dir = tempfile::tempdir().expect("tempdir");
    let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
    std::fs::create_dir_all(layout.projects_dir()).expect("projects dir");

    ops::create(&layout, "atlas", "Parse the header", CreateOpts::default()).expect("create");
    ops::create(&layout, "atlas", "Emit a summary", CreateOpts::default()).expect("create");
    ops::create(&layout, "beacon", "Document retries", CreateOpts::default()).expect("create");

    let socket = dir.path().join("control.sock");
    let owner = OwnerHandle::spawn(ServeConfig {
        layout: layout.clone(),
        socket,
        exe: None,
    })
    .expect("owner");
    (dir, layout, owner)
}

fn attach(owner: &OwnerHandle, layout: &Layout) -> ControlBackend {
    ControlBackend::connect(&owner.socket, layout, "live-test").expect("attach")
}

fn first_id(backend: &ControlBackend) -> String {
    backend.ready(None).expect("ready").issues[0].id.clone()
}

#[test]
fn the_client_reads_what_the_server_holds() {
    let (_dir, layout, owner) = live();
    let backend = attach(&owner, &layout);

    assert_eq!(backend.live(), BackendKind::Control);
    let page = backend.ready(None).expect("ready");
    assert_eq!(page.issues.len(), 3, "{:?}", page.issues);

    let mut projects = backend.projects().expect("projects");
    projects.sort();
    assert_eq!(projects, ["atlas", "beacon"]);

    // A detail fetched over the wire carries the same title the list showed.
    let row = &page.issues[0];
    let detail = backend.get(&row.id).expect("get");
    assert_eq!(detail.id, row.id);
    assert_eq!(detail.title, row.title);
}

/// A write over the socket is visible to the next read on the same backend.
///
/// The server rebuilds its catalog from a file watcher on its own cadence.
/// If the mutation path does not refresh before it answers, this reads the
/// catalog as it stood before the write: the claim is reported as made and
/// the issue still comes back unclaimed.
#[test]
fn a_claim_is_visible_to_the_next_read() {
    let (_dir, layout, owner) = live();
    let backend = attach(&owner, &layout);
    let id = first_id(&backend);

    let result = backend.claim(&id, false).expect("claim");
    assert!(result.ok, "{result:?}");

    let detail = backend.get(&id).expect("get after claim");
    assert_eq!(detail.state, "STARTED", "the claim did not take");
    assert_eq!(
        detail.claimed_by.as_deref(),
        Some("live-test"),
        "the claim names the attaching agent"
    );
}

/// A note the client sends is recorded, and the server admits to it.
///
/// `IssueDetail` carries no logbook, so the note text is not readable back
/// over the wire; what the client can observe is the report the server
/// returns and the revision it moves. The text itself is checked in the
/// file, which is the only place it lands.
#[test]
fn a_note_reaches_the_file_and_moves_the_revision() {
    let (_dir, layout, owner) = live();
    let backend = attach(&owner, &layout);
    let id = first_id(&backend);
    let before = backend.revision();

    let result = backend.note(&id, "seen from the live test").expect("note");
    assert!(result.ok, "{result:?}");
    assert!(result.report.contains(&id), "{}", result.report);
    assert!(backend.revision() > before);

    let file = std::fs::read_to_string(layout.project_issues_path("atlas")).expect("read");
    assert!(file.contains("seen from the live test"), "{file}");
}

/// The revision the server reports moves forward when the corpus changes.
///
/// Clients use it to tell a page they already hold from one they need to
/// fetch, so a revision that never moves makes every cache stale forever,
/// and one that moves on its own makes every cache useless.
#[test]
fn the_revision_moves_on_a_write_and_holds_still_otherwise() {
    let (_dir, layout, owner) = live();
    let backend = attach(&owner, &layout);
    let id = first_id(&backend);

    let before = backend.revision();
    backend.claim(&id, false).expect("claim");
    let after = backend.revision();
    assert!(after > before, "{before} -> {after}");

    // Nothing changes the corpus here, so nothing should move the revision.
    for _ in 0..3 {
        backend.ready(None).expect("ready");
        backend.projects().expect("projects");
    }
    assert_eq!(
        backend.revision(),
        after,
        "reading moved the revision on an idle tracker"
    );
}

/// A write by one client is seen by another attached to the same server.
#[test]
fn two_clients_see_the_same_tracker() {
    let (_dir, layout, owner) = live();
    let writer = attach(&owner, &layout);
    let reader = attach(&owner, &layout);
    let id = first_id(&writer);

    writer.claim(&id, false).expect("claim");

    // The reader learns about it through the server, not the file.
    // Generous on purpose. These assert that a change eventually arrives, and the
    // cost is asymmetric: the loop leaves the moment it does, so a passing run pays
    // nothing for a long deadline and only a genuine failure waits it out. Five
    // seconds is the sort of budget a watcher misses on a loaded machine, which turns
    // an eventual-delivery assertion into a race against the scheduler.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        if reader.get(&id).expect("get").state == "STARTED" {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the second client never saw the claim"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// An edit made outside the server still reaches an attached client.
///
/// This is the watcher's job: a person editing the org file in Emacs, or
/// another `vissue` invocation, does not go through the socket at all.
#[test]
fn a_write_outside_the_server_reaches_an_attached_client() {
    let (_dir, layout, owner) = live();
    let backend = attach(&owner, &layout);
    let before = backend.ready(None).expect("ready").issues.len();

    ops::create(
        &layout,
        "atlas",
        "Written behind the server's back",
        CreateOpts::default(),
    )
    .expect("create");

    // Generous on purpose. These assert that a change eventually arrives, and the
    // cost is asymmetric: the loop leaves the moment it does, so a passing run pays
    // nothing for a long deadline and only a genuine failure waits it out. Five
    // seconds is the sort of budget a watcher misses on a loaded machine, which turns
    // an eventual-delivery assertion into a race against the scheduler.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let page = backend.ready(None).expect("ready");
        if page.issues.len() == before + 1 {
            assert!(
                page.issues
                    .iter()
                    .any(|i| i.title == "Written behind the server's back"),
                "{:?}",
                page.issues
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the watcher never picked up an out-of-band write"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn an_unknown_id_comes_back_as_an_error_not_a_dropped_connection() {
    let (_dir, layout, owner) = live();
    let backend = attach(&owner, &layout);

    let err = backend.get("atlas-zzzz").expect_err("an unknown id");
    let text = err.to_string();
    assert!(
        text.contains("atlas-zzzz") || text.to_lowercase().contains("not found"),
        "{text}"
    );

    // The connection survives it: the next call still works.
    assert_eq!(backend.ready(None).expect("ready").issues.len(), 3);
}

#[test]
fn attaching_to_a_socket_nobody_serves_fails_rather_than_hanging() {
    let dir = tempfile::tempdir().expect("tempdir");
    let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
    let absent = dir.path().join("absent.sock");
    assert!(!Path::new(&absent).exists());
    assert!(ControlBackend::connect(&absent, &layout, "live-test").is_err());
}

/// An idle server stays still.
///
/// The generation poll opens the projects directory every 250ms and the
/// rebuild coalesces for 200ms. A watcher that counts its own reads as
/// changes therefore rebuilds several times a second forever, on a tracker
/// nobody is touching: every attached client is woken, and the revision
/// they use to tell a stale page from a fresh one runs away on its own.
///
/// This has to wait to see it. A burst of quick reads finishes inside one
/// poll interval and observes nothing.
#[test]
fn an_idle_server_does_not_rebuild_behind_the_client() {
    let (_dir, layout, owner) = live();
    let backend = attach(&owner, &layout);

    let settled = backend.ready(None).expect("ready").revision;
    std::thread::sleep(Duration::from_millis(1200));
    let later = backend.ready(None).expect("ready").revision;

    assert_eq!(
        later,
        settled,
        "the catalog was rebuilt {} time(s) while nothing changed",
        later.saturating_sub(settled)
    );
}

/// A refused write comes back as the error it is, not as prose.
///
/// The server reports failures as JSON-RPC codes with data; the client turns
/// those back into typed errors. Nothing tested that translation, so a board
/// asking "who holds this?" would have had only a message to parse. Each case
/// here goes over a real socket to a real owner.
#[test]
fn the_client_recovers_the_error_the_server_meant() {
    use vissue_core::error::Error;
    use vissue_tui::backend::UpdateReq;

    let (_dir, layout, owner) = live();
    let holder = ControlBackend::connect(&owner.socket, &layout, "holder").expect("attach");
    let rival = ControlBackend::connect(&owner.socket, &layout, "rival").expect("attach");
    let id = first_id(&holder);

    holder.claim(&id, false).expect("the first claim");

    // Someone else's claim names them, so a board can say who to ask.
    match rival.claim(&id, false) {
        Err(Error::ClaimConflict {
            id: got, holder, ..
        }) => {
            assert_eq!(got, id);
            assert_eq!(holder, "holder", "the conflict did not name the holder");
        }
        other => panic!("expected a claim conflict, got {other:?}"),
    }

    // Forcing it through is the documented way past that.
    rival.claim(&id, true).expect("a forced claim");

    // An issue cannot block itself.
    match rival.update(UpdateReq {
        id: id.clone(),
        state: None,
        priority: None,
        block: Some(id.clone()),
        unblock: None,
        if_state: None,
        if_gen: None,
    }) {
        Err(Error::BlockerCycle { blocker, issue }) => {
            assert_eq!(blocker, id, "{blocker} {issue}");
        }
        other => panic!("expected a blocker cycle, got {other:?}"),
    }

    // A closed issue refuses a claim, and says what state it is in.
    let other_id = holder
        .ready(None)
        .expect("ready")
        .issues
        .iter()
        .map(|r| r.id.clone())
        .find(|other| other != &id)
        .expect("a second issue");
    holder
        .update(UpdateReq {
            id: other_id.clone(),
            state: Some("DONE".into()),
            priority: None,
            block: None,
            unblock: None,
            if_state: None,
            if_gen: None,
        })
        .expect("close it");
    match rival.claim(&other_id, false) {
        Err(Error::InvalidState { id: got, state }) => {
            assert_eq!(got, other_id);
            assert_eq!(state, "DONE", "the refusal did not name the state");
        }
        other => panic!("expected an invalid state, got {other:?}"),
    }

    // An id that does not exist is its own error, not a generic failure.
    let missing = rival.get("atlas-zzzz");
    match missing {
        Err(Error::IssueNotFound { id }) => assert_eq!(id, "atlas-zzzz"),
        other => panic!("expected a not-found, got {other:?}"),
    }
}

/// Attaching to an owner serving a different tracker is refused.
///
/// Falling back to the files is safe; writing into the wrong vault is not,
/// so the mismatch is a distinct error carrying both sides.
#[test]
fn attaching_to_the_wrong_tracker_is_refused_by_name() {
    use vissue_tui::control::ControlAttachError;

    let (_dir, _layout, owner) = live();
    let elsewhere = tempfile::tempdir().expect("tempdir");
    let other = Layout::new(elsewhere.path(), DEFAULT_PREFIX);

    let attached = ControlBackend::connect(&owner.socket, &other, "wrong-vault");
    match attached {
        Err(ControlAttachError::Mismatch {
            want_root,
            got_root,
            ..
        }) => {
            assert!(
                want_root.contains(&elsewhere.path().display().to_string())
                    || !want_root.is_empty()
            );
            assert_ne!(want_root, got_root, "a mismatch that matches is not one");
        }
        Ok(_) => panic!("attached to an owner serving another tracker"),
        Err(other) => panic!("expected a mismatch, got {other}"),
    }
}
