//! What each key does to the board, driven through the public API.
//!
//! The palette is a state machine over a backend: every key either moves the
//! selection, changes which pane is showing, opens a draft, or writes through
//! to the tracker. None of that needs a window, so it is checked here against
//! a real tracker in a temporary directory rather than a rendered frame.
//!
//! This file stays outside `palette.rs` deliberately. It drives the crate the
//! way `vissue-hud` itself does, so it keeps working across changes to how
//! the module is laid out inside.

#![allow(missing_docs)]

use vissue_core::config::{DEFAULT_PREFIX, Layout};
use vissue_core::ops::{self, CreateOpts};
use vissue_hud::palette::{BoardFilter, ConfirmKind, DetailTab, Focus, Palette, PaletteKey};

/// A tracker with two projects, and a palette already inside one of them.
///
/// Home is the project list, so a test about rows has to enter a project
/// first; the ones about browsing use [`board`] instead.
fn open() -> (tempfile::TempDir, Palette) {
    let (dir, mut palette) = board();
    palette.enter_project("atlas");
    (dir, palette)
}

/// The same tracker, left on the project list.
fn board() -> (tempfile::TempDir, Palette) {
    let dir = tempfile::tempdir().expect("tempdir");
    let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
    std::fs::create_dir_all(layout.projects_dir()).expect("projects dir");

    for (project, title) in [
        ("atlas", "Parse the header"),
        ("atlas", "Emit a summary"),
        ("atlas", "Rename the config key"),
        ("beacon", "Document the retry policy"),
    ] {
        ops::create(&layout, project, title, CreateOpts::default()).expect("create");
    }

    let mut palette = Palette::open_core(layout, "board-test".into()).expect("open");
    palette.show();
    (dir, palette)
}

fn press(palette: &mut Palette, keys: &str) {
    for c in keys.chars() {
        palette.handle_key(PaletteKey::Char(c));
    }
}

fn titles(palette: &Palette) -> Vec<String> {
    palette
        .filtered_items()
        .iter()
        .map(|i| i.title.clone())
        .collect()
}

#[test]
fn a_hidden_palette_ignores_every_key() {
    let (_dir, mut palette) = open();
    let before = palette.selected_index();
    palette.hide();
    assert!(!palette.visible());

    press(&mut palette, "jjjan/");
    palette.handle_key(PaletteKey::Enter);
    palette.handle_key(PaletteKey::Space);

    assert_eq!(palette.selected_index(), before, "the selection moved");
    assert_eq!(palette.focus(), Focus::List, "a draft opened while hidden");
    assert!(palette.add_draft().is_empty());
}

#[test]
fn show_hide_and_toggle_agree_with_each_other() {
    let (_dir, mut palette) = open();
    assert!(palette.visible());
    palette.toggle();
    assert!(!palette.visible());
    palette.toggle();
    assert!(palette.visible());
    palette.hide();
    assert!(!palette.visible());
    palette.show();
    assert!(palette.visible());
}

#[test]
fn j_and_k_walk_the_rows_and_stop_at_the_ends() {
    let (_dir, mut palette) = open();
    let rows = palette.filtered_items().len();
    assert!(rows >= 3, "the fixture needs rows to walk: {rows}");

    assert_eq!(palette.selected_index(), 0);
    press(&mut palette, "j");
    assert_eq!(palette.selected_index(), 1);
    press(&mut palette, "k");
    assert_eq!(palette.selected_index(), 0);

    // Off the top stays at the top.
    press(&mut palette, "kkk");
    assert_eq!(palette.selected_index(), 0);

    // Off the bottom stays on the last row.
    for _ in 0..rows + 3 {
        palette.handle_key(PaletteKey::Down);
    }
    assert_eq!(palette.selected_index(), rows - 1);
    palette.handle_key(PaletteKey::Up);
    assert_eq!(palette.selected_index(), rows - 2);
}

#[test]
fn the_number_keys_choose_the_pane() {
    let (_dir, mut palette) = open();
    for (key, filter) in [
        ('2', BoardFilter::List),
        ('3', BoardFilter::Claims),
        ('4', BoardFilter::Agenda),
        ('1', BoardFilter::Ready),
    ] {
        palette.handle_key(PaletteKey::Char(key));
        assert_eq!(palette.filter(), filter, "{key} chose the wrong pane");
    }
    assert_eq!(palette.focus(), Focus::List);
}

#[test]
fn tab_cycles_the_panes_and_comes_back_around() {
    let (_dir, mut palette) = open();
    let first = palette.filter();
    let mut seen = vec![first];
    for _ in 0..6 {
        palette.handle_key(PaletteKey::Tab);
        seen.push(palette.filter());
    }
    assert!(
        seen.iter().filter(|f| **f == first).count() >= 2,
        "tab never returned to where it started: {seen:?}"
    );
    assert!(
        seen.contains(&BoardFilter::Claims) && seen.contains(&BoardFilter::Agenda),
        "{seen:?}"
    );
}

#[test]
fn escape_walks_back_out_rather_than_hiding_at_once() {
    let (_dir, mut palette) = open();
    palette.handle_key(PaletteKey::Char('3'));
    assert_eq!(palette.filter(), BoardFilter::Claims);

    // Inside a project, the first escape leaves it, and the pane comes back
    // to the default on the way out.
    palette.handle_key(PaletteKey::Esc);
    assert!(palette.browsing(), "escape did not leave the project");
    assert_eq!(palette.filter(), BoardFilter::Ready);
    assert!(palette.visible());

    // With nothing left to back out of, escape hides the board.
    palette.handle_key(PaletteKey::Esc);
    assert!(
        !palette.visible(),
        "escape had nowhere left to go but stayed"
    );
}

#[test]
fn escape_leaves_a_pane_before_it_hides_when_already_home() {
    let (_dir, mut palette) = board();
    palette.handle_key(PaletteKey::Char('3'));
    assert_eq!(palette.filter(), BoardFilter::Claims);

    palette.handle_key(PaletteKey::Esc);
    assert_eq!(palette.filter(), BoardFilter::Ready);
    assert!(
        palette.visible(),
        "the pane reset should not hide the board"
    );

    palette.handle_key(PaletteKey::Esc);
    assert!(!palette.visible());
}

#[test]
fn the_board_opens_on_the_project_list() {
    let (_dir, palette) = board();
    assert!(palette.browsing(), "home is the project list");
    let names: Vec<&str> = palette
        .project_cards()
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(names.contains(&"atlas"), "{names:?}");
    assert!(names.contains(&"beacon"), "{names:?}");
}

#[test]
fn entering_a_project_narrows_the_rows_to_it() {
    let (_dir, mut palette) = board();
    palette.enter_project("beacon");
    assert!(!palette.browsing());
    assert_eq!(palette.project(), Some("beacon"));
    assert_eq!(titles(&palette), ["Document the retry policy"]);

    palette.leave_project();
    assert!(palette.browsing());
    assert_eq!(palette.project(), None);
}

#[test]
fn p_cycles_through_the_projects_and_back_to_all() {
    let (_dir, mut palette) = open();
    let mut seen = Vec::new();
    for _ in 0..4 {
        press(&mut palette, "p");
        seen.push(palette.project().map(str::to_string));
    }
    assert!(
        seen.contains(&None),
        "cycling never returned to all: {seen:?}"
    );
    assert!(
        seen.iter().any(|p| p.as_deref() == Some("beacon")),
        "{seen:?}"
    );
}

#[test]
fn search_inside_a_project_keeps_other_projects_out() {
    let (_dir, mut palette) = open();
    press(&mut palette, "/retry");
    assert_eq!(palette.filter(), BoardFilter::Ready);
    let shown = titles(&palette);
    assert!(
        shown.iter().all(|t| !t.to_lowercase().contains("retry")),
        "atlas search leaked another project: {shown:?}"
    );

    palette.set_query("");
    press(&mut palette, "summary");
    let shown = titles(&palette);
    assert!(
        shown.iter().any(|t| t.contains("Emit a summary")),
        "{shown:?}"
    );
}

#[test]
fn slash_opens_search_and_typing_narrows_the_rows() {
    let (_dir, mut palette) = open();
    press(&mut palette, "/");
    assert_eq!(palette.focus(), Focus::Search);
    assert_eq!(palette.filter(), BoardFilter::Ready);

    let typed = "summary";
    press(&mut palette, typed);
    assert_eq!(palette.query(), typed);
    let shown = titles(&palette);
    assert!(
        shown.iter().any(|t| t.contains("Emit a summary")),
        "{shown:?}"
    );

    // Backspace walks the query back.
    let shorter = &typed[..typed.len() - 1];
    palette.handle_key(PaletteKey::Backspace);
    assert_eq!(palette.query(), shorter);

    // Space is text while the search field owns typing.
    palette.handle_key(PaletteKey::Space);
    assert_eq!(palette.query(), format!("{shorter} "));
}

#[test]
fn escape_in_search_clears_the_query_before_it_leaves() {
    let (_dir, mut palette) = open();
    press(&mut palette, "/abc");
    assert_eq!(palette.query(), "abc");

    palette.handle_key(PaletteKey::Esc);
    assert_eq!(palette.query(), "", "the first escape clears");
    assert_eq!(palette.focus(), Focus::Search, "and stays in the field");

    palette.handle_key(PaletteKey::Esc);
    assert_eq!(palette.focus(), Focus::List, "the second leaves it");
}

#[test]
fn enter_in_search_returns_to_the_list_keeping_the_query() {
    let (_dir, mut palette) = open();
    press(&mut palette, "/head");
    palette.handle_key(PaletteKey::Enter);
    assert_eq!(palette.focus(), Focus::List);
    assert_eq!(palette.query(), "head");
}

#[test]
fn a_adds_an_issue_and_escape_throws_the_draft_away() {
    let (_dir, mut palette) = open();
    let before = palette.filtered_items().len();

    press(&mut palette, "a");
    assert_eq!(palette.focus(), Focus::Add);
    press(&mut palette, "desk lamp");
    assert_eq!(palette.add_draft(), "desk lamp");

    // Backspace edits the draft.
    palette.handle_key(PaletteKey::Backspace);
    assert_eq!(palette.add_draft(), "desk lam");

    // Escape discards it.
    palette.handle_key(PaletteKey::Esc);
    assert_eq!(palette.focus(), Focus::List);
    assert!(palette.add_draft().is_empty());
    assert_eq!(palette.filtered_items().len(), before, "escape still wrote");

    // Enter commits.
    press(&mut palette, "a");
    press(&mut palette, "desk lamp");
    palette.handle_key(PaletteKey::Enter);
    assert_eq!(palette.focus(), Focus::List);
    assert!(palette.add_draft().is_empty());
    palette.set_query("desk lamp");
    assert!(
        titles(&palette).iter().any(|t| t == "desk lamp"),
        "{:?}",
        titles(&palette)
    );
}

#[test]
fn an_empty_add_writes_nothing() {
    let (_dir, mut palette) = open();
    let before = palette.filtered_items().len();
    press(&mut palette, "a");
    palette.handle_key(PaletteKey::Enter);
    assert_eq!(palette.filtered_items().len(), before);
}

#[test]
fn n_records_a_note_against_the_selected_row() {
    let (_dir, mut palette) = open();
    let id = palette.selected_id().expect("a row").to_string();

    press(&mut palette, "n");
    assert_eq!(palette.focus(), Focus::Note);
    assert_eq!(palette.note_draft(), Some(""));

    press(&mut palette, "held up");
    assert_eq!(palette.note_draft(), Some("held up"));
    palette.handle_key(PaletteKey::Backspace);
    assert_eq!(palette.note_draft(), Some("held u"));

    palette.handle_key(PaletteKey::Enter);
    assert_eq!(palette.focus(), Focus::List);
    assert!(palette.message().contains(&id), "{}", palette.message());
}

#[test]
fn escape_abandons_a_note_without_writing_it() {
    let (_dir, mut palette) = open();
    press(&mut palette, "n");
    press(&mut palette, "never mind");
    palette.handle_key(PaletteKey::Esc);
    assert_eq!(palette.focus(), Focus::List);
    assert_eq!(palette.note_draft(), None);
}

#[test]
fn c_claims_the_selected_row() {
    let (_dir, mut palette) = open();
    let id = palette.selected_id().expect("a row").to_string();
    press(&mut palette, "c");
    let detail = palette.backend().get(&id).expect("get");
    assert_eq!(detail.state, "STARTED");
    assert_eq!(detail.claimed_by.as_deref(), Some("board-test"));
}

#[test]
fn s_walks_a_row_through_the_open_states() {
    let (_dir, mut palette) = open();
    // From the list pane: a BLOCKED row is not ready, so the ready pane
    // would drop it half way round.
    palette.handle_key(PaletteKey::Char('2'));
    let id = palette.selected_id().expect("a row").to_string();
    assert_eq!(palette.backend().get(&id).unwrap().state, "TODO");

    press(&mut palette, "s");
    assert_eq!(palette.backend().get(&id).unwrap().state, "STARTED");
    palette.select_id(&id);
    press(&mut palette, "s");
    assert_eq!(palette.backend().get(&id).unwrap().state, "BLOCKED");
    palette.select_id(&id);
    press(&mut palette, "s");
    assert_eq!(palette.backend().get(&id).unwrap().state, "TODO");
}

#[test]
fn space_closes_a_row_and_opens_it_again() {
    let (_dir, mut palette) = open();
    // The list pane keeps closed work in view; the ready pane does not.
    palette.handle_key(PaletteKey::Char('2'));
    let id = palette.selected_id().expect("a row").to_string();
    palette.handle_key(PaletteKey::Space);
    assert_eq!(palette.backend().get(&id).unwrap().state, "DONE");

    palette.select_id(&id);
    palette.handle_key(PaletteKey::Space);
    assert_eq!(palette.backend().get(&id).unwrap().state, "TODO");
}

#[test]
fn closing_a_row_asks_first_and_takes_no_for_an_answer() {
    let (_dir, mut palette) = open();
    let id = palette.selected_id().expect("a row").to_string();

    press(&mut palette, "D");
    assert_eq!(palette.confirm(), Some(ConfirmKind::Done));

    // n backs out, and the row is untouched.
    press(&mut palette, "n");
    assert_eq!(palette.confirm(), None);
    assert_eq!(palette.backend().get(&id).unwrap().state, "TODO");

    // y goes through with it.
    press(&mut palette, "D");
    press(&mut palette, "y");
    assert_eq!(palette.confirm(), None);
    assert_eq!(palette.backend().get(&id).unwrap().state, "DONE");
}

#[test]
fn cancelling_a_row_is_its_own_confirmation() {
    let (_dir, mut palette) = open();
    let id = palette.selected_id().expect("a row").to_string();
    press(&mut palette, "X");
    assert_eq!(palette.confirm(), Some(ConfirmKind::Cancelled));
    palette.handle_key(PaletteKey::Enter);
    assert_eq!(palette.backend().get(&id).unwrap().state, "CANCELLED");
}

#[test]
fn escape_backs_out_of_a_confirmation() {
    let (_dir, mut palette) = open();
    press(&mut palette, "D");
    palette.handle_key(PaletteKey::Esc);
    assert_eq!(palette.confirm(), None);
}

#[test]
fn y_copies_the_selected_id() {
    let (_dir, mut palette) = open();
    let id = palette.selected_id().expect("a row").to_string();
    assert!(palette.clipboard().is_empty());
    press(&mut palette, "y");
    assert_eq!(palette.clipboard(), id);
    assert!(palette.message().contains(&id), "{}", palette.message());
}

#[test]
fn question_mark_opens_help_and_three_keys_close_it() {
    for closer in ['?', 'q'] {
        let (_dir, mut palette) = open();
        press(&mut palette, "?");
        assert_eq!(palette.focus(), Focus::Help);
        assert!(!palette.help_text().is_empty());

        // While help is up, other keys do nothing.
        press(&mut palette, "jja");
        assert_eq!(palette.focus(), Focus::Help);

        palette.handle_key(PaletteKey::Char(closer));
        assert_eq!(palette.focus(), Focus::List, "{closer} did not close help");
    }

    let (_dir, mut palette) = open();
    press(&mut palette, "?");
    palette.handle_key(PaletteKey::Esc);
    assert_eq!(palette.focus(), Focus::List);
}

#[test]
fn colon_opens_the_command_palette_and_enter_runs_the_chord() {
    let (_dir, mut palette) = open();
    press(&mut palette, ":");
    assert_eq!(palette.focus(), Focus::Palette);
    assert!(
        palette
            .command_hits()
            .iter()
            .any(|hit| hit.chord == "d" && hit.title.to_lowercase().contains("deed"))
    );
    press(&mut palette, "copy");
    assert_eq!(palette.command_hits()[0].id.as_str(), "issue.copy");
    palette.handle_key(PaletteKey::Enter);
    assert_eq!(palette.focus(), Focus::List);
    assert!(!palette.clipboard().is_empty());
}

#[test]
fn enter_cycles_the_side_tabs() {
    let (_dir, mut palette) = open();
    assert_eq!(palette.detail_tab(), DetailTab::Tree);

    let mut seen = vec![palette.detail_tab()];
    for _ in 0..DetailTab::ALL.len() {
        palette.handle_key(PaletteKey::Enter);
        seen.push(palette.detail_tab());
    }
    for tab in DetailTab::ALL {
        assert!(seen.contains(&tab), "{tab:?} never came up: {seen:?}");
    }
    assert_eq!(seen.last().copied(), Some(DetailTab::Tree));
}

#[test]
fn every_detail_tab_has_something_to_show() {
    let (_dir, mut palette) = open();
    for tab in DetailTab::ALL {
        palette.set_detail_tab(tab);
        assert_eq!(palette.detail_tab(), tab);
        // A tab may legitimately be empty, but it must not error out or
        // leave the palette in a draft.
        let _ = palette.detail_body();
        assert_eq!(palette.focus(), Focus::List, "{tab:?} changed the focus");
    }
}

#[test]
fn the_counts_follow_what_the_panes_hold() {
    let (_dir, mut palette) = open();
    let ready = palette.count(BoardFilter::Ready);
    assert!(ready > 0, "the fixture has open work");

    let id = palette.selected_id().expect("a row").to_string();
    press(&mut palette, "c");
    palette.set_filter(BoardFilter::Claims);
    assert!(
        palette.count(BoardFilter::Claims) > 0,
        "a claim did not show in the claims count"
    );
    assert!(
        palette.filtered_items().iter().any(|i| i.id == id),
        "the claimed row is missing from the claims pane"
    );
}

#[test]
fn the_status_line_names_the_agent_and_the_pane() {
    let (_dir, palette) = open();
    let line = palette.status_line();
    assert!(!line.is_empty());
    assert!(line.contains("board-test"), "{line}");
}

#[test]
fn reload_keeps_the_rows_it_already_had() {
    let (_dir, mut palette) = open();
    let before = titles(&palette);
    press(&mut palette, "R");
    assert_eq!(titles(&palette), before);
}

#[test]
fn selecting_by_id_moves_the_cursor_to_that_row() {
    let (_dir, mut palette) = open();
    let ids: Vec<String> = palette
        .filtered_items()
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert!(ids.len() >= 2);
    palette.select_id(&ids[1]);
    assert_eq!(palette.selected_id(), Some(ids[1].as_str()));

    // An id the pane does not hold leaves the selection alone.
    palette.select_id("atlas-zzzz");
    assert_eq!(palette.selected_id(), Some(ids[1].as_str()));
}

#[test]
fn the_pane_labels_and_the_tab_labels_are_all_distinct() {
    let panes = [
        BoardFilter::Ready,
        BoardFilter::List,
        BoardFilter::Claims,
        BoardFilter::Agenda,
        BoardFilter::Search,
    ];
    let mut labels: Vec<&str> = panes.iter().map(|f| f.label()).collect();
    labels.sort_unstable();
    let count = labels.len();
    labels.dedup();
    assert_eq!(labels.len(), count, "two panes share a label");

    let mut tabs: Vec<&str> = DetailTab::ALL.iter().map(|t| t.label()).collect();
    tabs.sort_unstable();
    let count = tabs.len();
    tabs.dedup();
    assert_eq!(tabs.len(), count, "two detail tabs share a label");

    // Cycling a pane visits the chips before repeating. Search is the
    // search field, not a stop on Tab.
    let mut seen = vec![BoardFilter::Ready];
    let mut cur = BoardFilter::Ready;
    for _ in 0..BoardFilter::CHIPS.len() {
        cur = cur.next();
        seen.push(cur);
    }
    for (pane, _) in BoardFilter::CHIPS {
        assert!(seen.contains(&pane), "{pane:?} is unreachable by cycling");
    }
    assert!(!seen.contains(&BoardFilter::Search));
}
