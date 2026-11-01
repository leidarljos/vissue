//! Board state that is not reached through a keystroke.
//!
//! Grouping rows into project sections, collapsing them, the excerpt and
//! detail panes, the drafts driven directly rather than typed, and what the
//! summon socket does to visibility. The key handling is in `board_keys.rs`.

#![allow(missing_docs)]

use vissue_core::config::{DEFAULT_PREFIX, Layout};
use vissue_core::ops::{self, CreateOpts};
use vissue_hud::palette::{BoardFilter, BoardRow, DetailTab, Focus, Palette, PaletteKey};
use vissue_hud::summon::{SummonAction, SummonRequest};

fn tracker() -> (tempfile::TempDir, Layout) {
    let dir = tempfile::tempdir().expect("tempdir");
    let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
    std::fs::create_dir_all(layout.projects_dir()).expect("projects dir");
    for (project, title) in [
        ("atlas", "Parse the header"),
        ("atlas", "Emit a summary"),
        ("beacon", "Document the retry policy"),
        ("beacon", "Retry with backoff"),
    ] {
        ops::create(&layout, project, title, CreateOpts::default()).expect("create");
    }
    (dir, layout)
}

/// A palette showing rows from more than one project at once.
///
/// With no project chosen, the ready and list panes are the project browser
/// and hold no rows at all. The search pane is where rows from across the
/// tracker appear together, so sections are worth grouping.
fn across_projects() -> (tempfile::TempDir, Palette) {
    let (dir, layout) = tracker();
    let mut palette = Palette::open_core(layout, "state-test".into()).expect("open");
    palette.show();
    palette.leave_project();
    palette.set_filter(BoardFilter::Search);
    palette.set_query("the");
    (dir, palette)
}

/// The project browser: no project chosen, on a pane that lists them.
fn browser() -> (tempfile::TempDir, Palette) {
    let (dir, layout) = tracker();
    let mut palette = Palette::open_core(layout, "state-test".into()).expect("open");
    palette.show();
    palette.leave_project();
    (dir, palette)
}

fn in_atlas() -> (tempfile::TempDir, Palette) {
    let (dir, layout) = tracker();
    let mut palette = Palette::open_core(layout, "state-test".into()).expect("open");
    palette.show();
    palette.enter_project("atlas");
    (dir, palette)
}

#[test]
fn rows_are_grouped_into_one_section_per_project() {
    let (_dir, palette) = across_projects();
    let sections = palette.sections();
    assert!(
        sections.len() >= 2,
        "the search pane holds both projects: {} section(s)",
        sections.len()
    );

    for section in &sections {
        assert!(!section.rows.is_empty(), "an empty section");
        assert_eq!(section.end - section.start, section.rows.len());
        for (_, item) in &section.rows {
            assert_eq!(
                item.project, section.project,
                "a row is filed under the wrong project"
            );
        }
    }

    // The sections cover every visible row exactly once, in order.
    let covered: usize = sections.iter().map(|s| s.rows.len()).sum();
    assert_eq!(covered, palette.filtered_items().len());
    let mut next = 0;
    for section in &sections {
        assert_eq!(section.start, next, "a gap between sections");
        next = section.end;
    }
}

#[test]
fn search_board_rows_are_headers_then_open_project_cards() {
    let (_dir, mut palette) = across_projects();
    let rows = palette.board_rows();
    assert!(
        rows.iter()
            .any(|row| matches!(row, BoardRow::Header { .. })),
        "grouped search paints a header per project"
    );
    assert!(
        rows.iter().any(|row| matches!(row, BoardRow::Task { .. })),
        "the open section still lists its cards"
    );
    assert_eq!(palette.task_heights().len(), rows.len());

    let open: Vec<String> = palette
        .sections()
        .iter()
        .filter(|section| !section.collapsed)
        .map(|section| section.project.to_string())
        .collect();
    let before = rows.len();
    for name in &open {
        palette.toggle_project(name);
    }
    let collapsed = palette.board_rows();
    assert!(
        collapsed
            .iter()
            .all(|row| matches!(row, BoardRow::Header { open: false, .. })),
        "collapsed groups keep the header only"
    );
    assert!(
        collapsed.len() < before,
        "hiding the open cards should shrink the board"
    );
    assert_eq!(palette.task_heights().len(), collapsed.len());
}

#[test]
fn moving_down_the_task_list_scrolls_the_window_to_the_cursor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
    std::fs::create_dir_all(layout.projects_dir()).expect("projects dir");
    for i in 0..40 {
        ops::create(
            &layout,
            "atlas",
            &format!("Task {i:02}"),
            CreateOpts::default(),
        )
        .expect("create");
    }
    let mut palette = Palette::open_core(layout, "virt".into()).expect("open");
    palette.show();
    palette.enter_project("atlas");
    palette.set_filter(BoardFilter::List);

    assert_eq!(palette.board_rows().len(), 40);
    assert_eq!(palette.task_heights().len(), 40);
    let first = palette.task_window();
    assert!(first.end > 0, "the first paint must mount a window");
    assert!(
        first.end < 40,
        "a short pane must not mount every card: {}..{}",
        first.start,
        first.end
    );

    for _ in 0..30 {
        palette.handle_key(PaletteKey::Down);
    }
    let selected = palette.selected_index();
    let cover = palette
        .board_rows()
        .iter()
        .position(|row| matches!(row, BoardRow::Task { index, .. } if *index == selected))
        .expect("cursor on a card");
    let win = palette.task_window();
    assert!(
        cover >= win.start && cover < win.end,
        "cursor {cover} outside window {}..{}",
        win.start,
        win.end
    );
    assert!(
        win.scroll > 0.0 || win.start > 0,
        "the window must move with the cursor"
    );
}

#[test]
fn collapsing_a_project_is_a_toggle() {
    let (_dir, mut palette) = across_projects();
    let name = palette.sections()[0].project.to_string();

    let was = palette
        .sections()
        .iter()
        .find(|s| s.project == name)
        .map(|s| s.collapsed)
        .expect("the section");

    palette.toggle_project(&name);
    let now = palette
        .sections()
        .iter()
        .find(|s| s.project == name)
        .map(|s| s.collapsed)
        .expect("the section");
    assert_ne!(was, now, "toggling did not change the section");

    palette.toggle_project(&name);
    let back = palette
        .sections()
        .iter()
        .find(|s| s.project == name)
        .map(|s| s.collapsed)
        .expect("the section");
    assert_eq!(back, was, "toggling twice did not come back");

    // Collapsing is a display state: the rows themselves are untouched.
    let rows: usize = palette.sections().iter().map(|s| s.rows.len()).sum();
    palette.toggle_project(&name);
    assert_eq!(
        palette
            .sections()
            .iter()
            .map(|s| s.rows.len())
            .sum::<usize>(),
        rows,
        "collapsing dropped rows"
    );
}

#[test]
fn the_project_cards_count_the_ready_work_in_each() {
    let (_dir, palette) = browser();
    let cards = palette.project_cards();
    assert!(cards.len() >= 2, "{} card(s)", cards.len());
    for card in cards {
        assert!(!card.name.is_empty());
        assert!(card.ready > 0, "{} has no ready work", card.name);
    }
    let total: usize = cards.iter().map(|c| c.ready).sum();
    assert_eq!(total, 4, "every issue in the fixture is ready");
}

#[test]
fn the_selected_project_tracks_the_cursor_on_the_card_list() {
    let (_dir, palette) = browser();
    assert!(palette.browsing());
    let first = palette.selected_project_name().map(str::to_string);
    assert!(first.is_some(), "a card is selected to begin with");
    assert_eq!(palette.selected_project_index(), 0);
}

#[test]
fn the_excerpt_pane_reads_the_issue_off_disk() {
    let (_dir, palette) = in_atlas();
    let excerpt = palette.excerpt().expect("an excerpt");
    assert!(!excerpt.text.is_empty(), "{excerpt:?}");
    assert!(excerpt.file.contains("issues.org"), "{excerpt:?}");
    assert!(!excerpt.suppressed);

    let detail = palette.detail().expect("a detail");
    assert_eq!(Some(detail.id.as_str()), palette.selected_id());
}

#[test]
fn the_detail_body_follows_the_tab() {
    let (_dir, mut palette) = in_atlas();
    for tab in DetailTab::ALL {
        palette.set_detail_tab(tab);
        assert!(
            palette.excerpt().is_some(),
            "{tab:?} dropped the issue table"
        );
        assert!(
            palette
                .detail_body()
                .contains(palette.selected_id().unwrap()),
            "{tab:?} dropped the issue id: {}",
            palette.detail_body()
        );
    }
}

#[test]
fn a_draft_set_directly_submits_the_same_way_a_typed_one_does() {
    let (_dir, mut palette) = in_atlas();
    let before = palette.filtered_items().len();

    palette.set_add_draft("wired directly");
    assert_eq!(palette.add_draft(), "wired directly");
    palette.submit_add();
    assert!(palette.add_draft().is_empty(), "the draft was not consumed");
    assert_eq!(palette.focus(), Focus::List);

    palette.set_query("wired directly");
    assert!(
        palette
            .filtered_items()
            .iter()
            .any(|i| i.title == "wired directly"),
        "the issue was not created"
    );
    palette.set_query("");
    assert_eq!(palette.filtered_items().len(), before + 1);
}

#[test]
fn a_note_set_directly_is_recorded_against_the_selection() {
    let (_dir, mut palette) = in_atlas();
    let id = palette.selected_id().expect("a row").to_string();

    palette.set_note_draft("wired directly");
    assert_eq!(palette.note_draft(), Some("wired directly"));
    palette.submit_note();
    assert!(palette.message().contains(&id), "{}", palette.message());

    let file = std::fs::read_to_string(palette.backend().layout().project_issues_path("atlas"))
        .expect("read");
    assert!(file.contains("wired directly"), "{file}");
}

#[test]
fn focus_helpers_move_between_the_fields() {
    let (_dir, mut palette) = in_atlas();
    assert_eq!(palette.focus(), Focus::List);

    palette.focus_search();
    assert_eq!(palette.focus(), Focus::Search);
    palette.focus_list();
    assert_eq!(palette.focus(), Focus::List);
    palette.focus_add();
    assert_eq!(palette.focus(), Focus::Add);
    palette.focus_list();
    assert_eq!(palette.focus(), Focus::List);
}

#[test]
fn the_summon_socket_drives_visibility() {
    let (_dir, mut palette) = in_atlas();

    palette.apply_summon(&SummonRequest::new(SummonAction::Hide));
    assert!(!palette.visible());
    palette.apply_summon(&SummonRequest::new(SummonAction::Show));
    assert!(palette.visible());
    palette.apply_summon(&SummonRequest::new(SummonAction::Toggle));
    assert!(!palette.visible());
    palette.apply_summon(&SummonRequest::new(SummonAction::Toggle));
    assert!(palette.visible());
}

#[test]
fn a_core_palette_reports_what_it_is_attached_to() {
    let (_dir, palette) = in_atlas();
    assert_eq!(palette.agent(), "state-test");
    // Nothing is serving, so the numbers come from the files themselves.
    assert_eq!(palette.backend().live(), vissue_tui::BackendKind::Core);
    let _ = palette.serve_status();
    let _ = palette.revision();
    assert!(
        palette.generation() > 0,
        "a written tracker has a generation"
    );
}

#[test]
fn polling_an_unchanged_tracker_leaves_the_rows_alone() {
    let (_dir, mut palette) = in_atlas();
    let before: Vec<String> = palette
        .filtered_items()
        .iter()
        .map(|i| i.id.clone())
        .collect();
    palette.poll_updates();
    let after: Vec<String> = palette
        .filtered_items()
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(before, after);
}

#[test]
fn toggling_done_from_the_api_writes_through_and_back() {
    let (_dir, mut palette) = in_atlas();
    palette.set_filter(BoardFilter::List);
    let id = palette.selected_id().expect("a row").to_string();

    palette.toggle_done(&id);
    assert_eq!(palette.backend().get(&id).unwrap().state, "DONE");
    palette.toggle_done(&id);
    assert_eq!(palette.backend().get(&id).unwrap().state, "TODO");
}

#[test]
fn the_help_text_names_the_keys_it_documents() {
    let (_dir, palette) = in_atlas();
    let help = palette.help_text();
    for fragment in ["claim", "note"] {
        assert!(
            help.to_lowercase().contains(fragment),
            "help does not mention {fragment}: {help}"
        );
    }
}

#[test]
fn an_empty_tracker_opens_without_rows_and_without_panic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
    std::fs::create_dir_all(layout.projects_dir()).expect("projects dir");

    let mut palette = Palette::open_core(layout, "state-test".into()).expect("open");
    palette.show();
    assert!(palette.project_cards().is_empty());
    assert_eq!(palette.selected_id(), None);

    // Actions that need a selection do nothing rather than fall over.
    palette.claim_selected();
    palette.show_excerpt();
    palette.submit_note();
    palette.set_add_draft("nowhere to put this");
    palette.submit_add();
    assert!(palette.visible());
}
