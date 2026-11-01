//! Board face on icedtea constructors. Drawing only; logic lives in [`crate::palette`].

use iced::widget::{Id, Space, button, column, container, mouse_area, row, text};
use iced::{Alignment, Element, Fill, Length};
use icedtea::a11y::{A11y, Role};
use icedtea::collection::Tabs;
use icedtea::i18n::Direction;
use icedtea::icon::Icons;
use icedtea::theme::Tokens;
use icedtea::toast::ToastKind;
use icedtea::typo::FontFace;
use icedtea::variant::Variant;
use icedtea::widget;

use crate::app::Message;
use crate::palette::{BoardFilter, BoardRow, DetailTab, Focus, HudItem, Palette, TreeRow};
use crate::theme;

/// Board face. Hidden state draws nothing so a closed overlay is empty.
pub fn view(palette: &Palette) -> Element<'_, Message> {
    if !palette.visible() {
        return Space::new().width(0).height(0).into();
    }
    let tea = theme::tokens();
    if palette.focus() == Focus::Help {
        return help_overlay(palette, tea);
    }
    if palette.focus() == Focus::Palette {
        return command_overlay(palette, tea);
    }

    let mut pane = column![header(palette, tea), actions(palette, tea)]
        .spacing(10)
        .padding([14, 16]);

    if palette.browsing() {
        pane = pane.push(project_browser(palette, tea));
    } else {
        let sections = palette.sections();
        if sections.is_empty() {
            pane = pane.push(icedtea::pattern::status_page(
                empty_copy(palette),
                "a to add, / to search",
                None,
                tea,
                A11y::new("empty", Role::Status),
            ));
        } else {
            pane = pane.push(task_board(palette, tea));
        }
        if sections.is_empty() && palette.painted_id().is_some() {
            pane = pane.push(detail_panel(palette, tea));
        }
    }

    if palette.note_draft().is_some() {
        pane = pane.push(note_bar(palette, tea));
    }
    if palette.deed_draft().is_some() {
        pane = pane.push(deed_bar(palette, tea));
    }
    if let Some(kind) = palette.confirm() {
        pane = pane.push(widget::banner(
            format!("confirm {}? y/n", kind.state()),
            None,
            Some(ToastKind::Warning),
            tea,
            A11y::new("confirm", Role::Status),
        ));
    }
    if !palette.message().is_empty() {
        pane = pane.push(widget::banner(
            palette.message().to_string(),
            None,
            Some(ToastKind::Info),
            tea,
            A11y::new("status", Role::Status),
        ));
    }

    container(pane.spacing(10).width(Fill).height(Fill))
        .width(Fill)
        .height(Fill)
        .style(move |_| icedtea::style::shell(tea))
        .into()
}

fn header(palette: &Palette, tea: Tokens) -> Element<'_, Message> {
    let (live, live_var) = match palette.serve_status() {
        vissue_tui::attach::ServeStatus::Live => ("live", Variant::Success),
        vissue_tui::attach::ServeStatus::Mismatch => ("mismatch", Variant::Warning),
        vissue_tui::attach::ServeStatus::Offline => ("offline", Variant::Quiet),
    };
    row![
        widget::label(
            "vissue",
            widget::LabelFace::Body,
            tea,
            A11y::new("vissue", Role::Header),
        ),
        widget::badge(
            live,
            None,
            tea,
            live_var,
            widget::BadgeSize::Small,
            A11y::new("serve", Role::Status),
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn actions(palette: &Palette, tea: Tokens) -> Element<'_, Message> {
    row![
        chips(palette, tea),
        container(find_control(palette, tea)).width(Fill),
        add_bar(palette, tea),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn find_control(palette: &Palette, tea: Tokens) -> Element<'_, Message> {
    widget::search_input(
        palette.query(),
        Message::QueryChanged,
        Some(Message::QueryChanged(String::new())),
        Some(Message::FocusList),
        tea,
        A11y::new("Search title, id, tags", Role::TextBox),
        None,
        &[],
    )
}

fn chips(palette: &Palette, tea: Tokens) -> Element<'_, Message> {
    let mut row = row![].spacing(6);
    if palette.browsing() {
        row = row.push(filter_chip(
            "Projects",
            palette.project_cards().len(),
            true,
            Message::LeaveProject,
            tea,
        ));
        for (filter, label) in [
            (BoardFilter::Claims, "Claims"),
            (BoardFilter::Agenda, "Agenda"),
        ] {
            row = row.push(filter_chip(
                label,
                palette.count(filter),
                false,
                Message::Filter(filter),
                tea,
            ));
        }
        return row.into();
    }
    row = row.push(filter_chip(
        "Projects",
        0,
        false,
        Message::LeaveProject,
        tea,
    ));
    for (filter, label) in BoardFilter::CHIPS {
        row = row.push(filter_chip(
            label,
            palette.count(filter),
            palette.filter() == filter,
            Message::Filter(filter),
            tea,
        ));
    }
    row.into()
}

fn filter_chip(
    label: &str,
    count: usize,
    active: bool,
    msg: Message,
    tea: Tokens,
) -> Element<'static, Message> {
    let title = if count > 0 {
        format!("{label} {count}")
    } else {
        label.to_string()
    };
    widget::chip(
        title.clone(),
        Some(msg),
        None,
        tea,
        if active {
            Variant::Primary
        } else {
            Variant::Chip
        },
        widget::ChipKind::Filter,
        Icons::NONE,
        A11y::button(title),
    )
}

fn project_browser(palette: &Palette, tea: Tokens) -> Element<'_, Message> {
    let cards = palette.project_cards();
    if cards.is_empty() {
        return icedtea::pattern::status_page(
            "No projects under this prefix.",
            "Point --prefix at a Software tree.",
            None,
            tea,
            A11y::new("no-projects", Role::Status),
        );
    }
    let muted = tea.muted;
    widget::list_view(
        palette.project_list(),
        palette.project_selection(),
        |click| Message::PickProject(click.id),
        tea,
        widget::ListOpts {
            window: palette.project_window(),
            row_h: 56.0,
            overscan: 2,
            on_scroll: Message::ProjectScroll,
            empty: "No projects under this prefix.",
            meta_color: move |_| muted,
            scroll_id: None,
            face: icedtea::collection::RowFace::Card {
                meter: None::<fn(usize) -> f32>,
            },
            on_check: |_| Message::Noop,
            // New in this version, and there was no context action before, so
            // the row keeps doing nothing on a right click.
            on_context: |_| Message::Noop,
        },
        A11y::new("projects", Role::List),
    )
}

fn task_board<'a>(palette: &'a Palette, tea: Tokens) -> Element<'a, Message> {
    let rows = palette.board_rows();
    let selected = palette.selected_index();
    let list = widget::virtual_column(
        palette.task_heights(),
        palette.task_window(),
        2,
        None,
        Message::TaskScroll,
        // A click handler is a parameter now. The list had none before, and a
        // row that suddenly answered a click would be a change nobody asked
        // for, so it stays inert.
        |_| Message::Noop,
        None,
        tea,
        move |i| match rows.get(i) {
            Some(BoardRow::Header {
                project,
                count,
                open,
                searching,
            }) => group_header(project, *count, *open, *searching, tea),
            Some(BoardRow::Task { index, item }) => task_row(item, *index == selected, tea),
            None => Space::new().height(0).into(),
        },
        A11y::new("tasks", Role::List),
    );
    let list = container(list).width(Fill).height(Fill).into();
    if palette.painted_id().is_some() {
        list_detail(palette, list, tea)
    } else {
        list
    }
}

fn add_bar(palette: &Palette, tea: Tokens) -> Element<'_, Message> {
    if palette.focus() == Focus::Add {
        widget::text_input(
            "Add a task in the current project",
            palette.add_draft(),
            Message::AddChanged,
            Some(Message::AddSubmit),
            widget::FieldOpts::NONE,
            tea,
            A11y::new("add", Role::TextBox),
            None,
        )
    } else {
        widget::button(
            "+ Add a task",
            Some(Message::FocusAdd),
            tea,
            Variant::Quiet,
            Icons::NONE,
            widget::ButtonOpts::SHRINK,
            A11y::button("Add a task"),
        )
    }
}

fn group_header(
    project: &str,
    count: usize,
    open: bool,
    searching: bool,
    tea: Tokens,
) -> Element<'static, Message> {
    let name = project.to_string();
    let twisty: Element<'static, Message> = text(tree_twisty_mark(open, tea.direction))
        .size(tea.meta())
        .color(tea.muted)
        .into();
    let title: Element<'static, Message> = text(project.to_string())
        .size(tea.body())
        .font(icedtea::typo::UI)
        .color(tea.text)
        .into();
    let count = badge_mark(
        count.to_string(),
        if searching {
            Variant::Primary
        } else {
            Variant::Chip
        },
        tea,
        "count",
    );
    let mut face = row![];
    for kid in icedtea::i18n::order(tea.direction, [twisty, title, count]) {
        face = face.push(kid);
    }
    mouse_area(
        container(
            face.spacing(8)
                .align_y(Alignment::Center)
                .width(Fill)
                .padding([8, 6]),
        )
        .width(Fill)
        .style(move |_| icedtea::style::card(tea, false)),
    )
    .on_press(Message::ToggleProject(name))
    .into()
}

/// Outline indent: four density gaps (16px at Compact), wider than the twisty.
pub(crate) fn tree_indent_px(depth: u32, tea: Tokens) -> f32 {
    depth as f32 * tea.density.gap() * 4.0
}

fn tree_twisty_width(tea: Tokens) -> f32 {
    tea.density.gap() * 2.0 + 4.0
}

fn tree_twisty_mark(expanded: bool, dir: Direction) -> &'static str {
    if expanded {
        "▾"
    } else {
        match dir {
            Direction::Ltr => "▸",
            Direction::Rtl => "◂",
        }
    }
}

fn tree_row<'a>(node: TreeRow<'a>, selected: bool, tea: Tokens) -> Element<'a, Message> {
    let twisty: Element<'a, Message> = if node.has_children {
        let mark = tree_twisty_mark(node.expanded, tea.direction);
        button(text(mark).size(tea.meta()).color(tea.muted))
            .padding((tea.density.gap() / 2.0).max(4.0))
            .style(icedtea::style::button_style(tea, Variant::Ghost))
            .on_press(Message::TreeToggle(node.tea_id))
            .into()
    } else {
        Space::new().width(tree_twisty_width(tea)).into()
    };
    let title_color = if matches!(node.state, "DONE" | "CANCELLED") {
        tea.muted
    } else {
        tea.text
    };
    let pick = node.tea_id;
    let open = node.issue_id.to_string();
    let indent: Element<'a, Message> = Space::new().width(tree_indent_px(node.depth, tea)).into();
    let state = badge_state(node.state, tea);
    let title: Element<'a, Message> = text(node.title.to_string())
        .size(tea.meta())
        .font(icedtea::typo::UI)
        .color(title_color)
        .wrapping(iced::widget::text::Wrapping::Word)
        .width(Fill)
        .into();
    let hit: Element<'a, Message> = {
        let mut face = row![];
        for kid in icedtea::i18n::order(tea.direction, [state, title]) {
            face = face.push(kid);
        }
        mouse_area(
            face.spacing(6)
                .align_y(Alignment::Center)
                .width(Fill)
                .padding(tea.density.gap() / 2.0),
        )
        .on_press(Message::TreePick(pick))
        .on_double_click(Message::OpenIssue(open))
        .into()
    };
    let mut line = row![];
    for kid in icedtea::i18n::order(tea.direction, [indent, twisty, hit]) {
        line = line.push(kid);
    }
    let line = line.spacing(4).align_y(Alignment::Center).width(Fill);
    container(line)
        .width(Fill)
        .style(move |_| icedtea::style::list_row(tea, selected))
        .into()
}

fn task_row(item: &HudItem, selected: bool, tea: Tokens) -> Element<'_, Message> {
    let done = item.state == "DONE";
    let can_toggle = matches!(item.state.as_str(), "TODO" | "DONE");
    let id = item.id.clone();
    let id_toggle = item.id.clone();
    let marks = issue_marks(
        Some(item.priority.as_str()),
        &item.state,
        !item.blocked_by.is_empty(),
        item.claimed_by.is_some(),
        tea,
    );
    let mut bits: Vec<String> = Vec::new();
    if let Some(parent) = item.parent.as_deref()
        && item.depth == 0
    {
        bits.push(format!("under {parent}"));
    }
    if !item.extra.is_empty() && item.claimed_by.is_none() {
        bits.push(item.extra.clone());
    }
    if let Some(due) = item.due.as_deref() {
        bits.push(due.to_string());
    }
    let title_color = if done { tea.muted } else { tea.text };
    let indent: Element<'_, Message> = Space::new()
        .width(item.depth as f32 * tea.density.gap() * 4.0)
        .into();
    let box_el = widget::checkbox(
        "",
        widget::CheckState::from(done),
        move |_| Message::ToggleDone(id_toggle.clone()),
        tea,
        A11y::new("done", Role::Checkbox).with_disabled(!can_toggle),
    );
    let title = text(item.title.clone())
        .size(tea.body())
        .font(icedtea::typo::UI)
        .color(title_color)
        .wrapping(iced::widget::text::Wrapping::Word)
        .width(Fill);
    let mut body_col = column![title].spacing(2).width(Fill);
    if !bits.is_empty() {
        body_col = body_col.push(
            text(bits.join("  ·  "))
                .size(tea.meta())
                .color(tea.muted)
                .wrapping(iced::widget::text::Wrapping::Word)
                .width(Fill),
        );
    }
    let project = project_mark(&item.project, tea);
    let hit: Element<'_, Message> = {
        let mut face = row![];
        for kid in icedtea::i18n::order(tea.direction, [marks, body_col.into(), project]) {
            face = face.push(kid);
        }
        mouse_area(face.spacing(8).align_y(Alignment::Start).width(Fill))
            .on_press(Message::SelectId(id))
            .into()
    };
    let mut line = row![];
    for kid in icedtea::i18n::order(tea.direction, [indent, box_el, hit]) {
        line = line.push(kid);
    }
    let body = line
        .spacing(8)
        .align_y(Alignment::Start)
        .width(Fill)
        .padding([8, 6]);
    container(body)
        .width(Fill)
        .style(move |_| icedtea::style::card(tea, selected))
        .into()
}

/// Extra "blocked" chip only when the state keyword is not already BLOCKED
/// and the heading is still open.
pub(crate) fn extra_blocked_mark(state: &str, blocked: bool) -> bool {
    blocked && matches!(state, "TODO" | "STARTED")
}

fn issue_marks(
    priority: Option<&str>,
    state: &str,
    blocked: bool,
    claimed: bool,
    tea: Tokens,
) -> Element<'static, Message> {
    let mut marks = row![].spacing(4).align_y(Alignment::Center);
    if let Some(priority) = priority {
        marks = marks.push(badge_priority(priority, tea));
    }
    marks = marks.push(badge_state(state, tea));
    if extra_blocked_mark(state, blocked) {
        marks = marks.push(badge_mark("blocked", Variant::Warning, tea, "blocked"));
    }
    if claimed {
        marks = marks.push(badge_mark("claimed", Variant::Primary, tea, "claimed"));
    }
    marks.into()
}

fn project_mark(project: &str, tea: Tokens) -> Element<'static, Message> {
    badge_mark(project.to_string(), Variant::Chip, tea, "project")
}

fn badge_priority(priority: &str, tea: Tokens) -> Element<'static, Message> {
    badge_mark(
        priority.to_string(),
        match priority {
            "A" => Variant::Danger,
            "B" => Variant::Warning,
            _ => Variant::Quiet,
        },
        tea,
        "priority",
    )
}

fn badge_state(state: &str, tea: Tokens) -> Element<'static, Message> {
    badge_mark(
        state.to_lowercase(),
        match state {
            "STARTED" => Variant::Primary,
            "BLOCKED" => Variant::Warning,
            "DONE" => Variant::Success,
            "CANCELLED" => Variant::Quiet,
            _ => Variant::Chip,
        },
        tea,
        "state",
    )
}

fn badge_mark(
    title: impl Into<String>,
    variant: Variant,
    tea: Tokens,
    name: &str,
) -> Element<'static, Message> {
    let title = title.into();
    widget::badge(
        title,
        None,
        tea,
        variant,
        widget::BadgeSize::Small,
        A11y::new(name, Role::Status),
    )
}

fn detail_panel(palette: &Palette, tea: Tokens) -> Element<'_, Message> {
    let main = column![issue_header(palette, tea), issue_main(palette, tea)]
        .spacing(8)
        .width(Fill)
        .height(Fill);
    let body = row![
        container(main).width(Length::FillPortion(3)).height(Fill),
        container(Space::new().width(1).height(Fill)).style(move |_| icedtea::style::hairline(tea)),
        container(side_pane(palette, tea))
            .width(Length::FillPortion(2))
            .height(Fill),
    ]
    .spacing(12)
    .width(Fill)
    .height(Fill);
    container(body.padding(12))
        .width(Fill)
        .height(Fill)
        .style(move |_| icedtea::style::panel(tea))
        .into()
}

fn issue_header(palette: &Palette, tea: Tokens) -> Element<'_, Message> {
    let Some(issue) = palette.header_issue() else {
        return Space::new().width(Fill).into();
    };
    let title_color = if issue.state == "DONE" {
        tea.muted
    } else {
        tea.text
    };
    column![
        row![
            issue_marks(
                Some(issue.priority),
                issue.state,
                issue.blocked,
                issue.claimed,
                tea,
            ),
            project_mark(issue.project, tea),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        text(issue.title.to_string())
            .size(tea.body())
            .font(icedtea::typo::UI)
            .color(title_color)
            .wrapping(iced::widget::text::Wrapping::Word)
            .width(Fill),
    ]
    .spacing(6)
    .width(Fill)
    .into()
}

fn side_pane<'a>(palette: &'a Palette, tea: Tokens) -> Element<'a, Message> {
    let titles: Vec<String> = DetailTab::ALL
        .iter()
        .map(|t| t.label().to_string())
        .collect();
    let active = DetailTab::ALL
        .iter()
        .position(|t| *t == palette.detail_tab())
        .unwrap_or(0);
    iced::widget::responsive(move |size| {
        let mut tabs = Tabs::new(titles.clone());
        tabs.select(active);
        tabs.closable = false;
        let bar = widget::tab_bar(
            &tabs,
            |i| Message::DetailTab(DetailTab::ALL[i.min(DetailTab::ALL.len() - 1)]),
            |_| Message::Noop,
            size.width,
            false,
            tea,
            A11y::new("detail", Role::Tab),
        );
        let content = pane_scroll(
            detail_tab_body(palette, tea),
            tea,
            A11y::new("detail-scroll", Role::Group),
        );
        column![bar, content].spacing(8).height(Fill).into()
    })
    .width(Fill)
    .height(Fill)
    .into()
}

fn list_detail<'a>(
    palette: &'a Palette,
    list: Element<'a, Message>,
    tea: Tokens,
) -> Element<'a, Message> {
    if palette.preview_hidden() {
        return list;
    }
    icedtea::layout::split_view(
        container(list).width(Fill).into(),
        container(detail_panel(palette, tea)).width(Fill).into(),
        palette.detail_split(),
        palette.split_total(),
        Message::Sash,
        tea.direction,
        tea,
    )
}

fn note_bar(palette: &Palette, tea: Tokens) -> Element<'_, Message> {
    let draft = palette.note_draft().unwrap_or("");
    widget::text_input(
        "Add a note to the logbook",
        draft,
        Message::NoteChanged,
        Some(Message::NoteSubmit),
        widget::FieldOpts::NONE,
        tea,
        A11y::new("note", Role::TextBox),
        None,
    )
}

fn deed_bar(palette: &Palette, tea: Tokens) -> Element<'_, Message> {
    let draft = palette.deed_draft().unwrap_or("");
    widget::text_input(
        "Cite a deed: deed-<kind>-<slug>, or sha256:...",
        draft,
        Message::DeedChanged,
        Some(Message::DeedSubmit),
        widget::FieldOpts::NONE,
        tea,
        A11y::new("deed", Role::TextBox),
        None,
    )
}

fn command_overlay(palette: &Palette, tea: Tokens) -> Element<'_, Message> {
    let hits = palette.command_hits();
    let selected = palette.command_selected();
    let mut list = column![].spacing(2).width(Fill);
    if hits.is_empty() {
        list = list.push(meta(
            "Nothing matches.",
            tea,
            A11y::new("empty", Role::Status),
        ));
    }
    for (i, hit) in hits.iter().enumerate() {
        let on = i == selected;
        let line = format!("{:<8}  {}", hit.chord, hit.title);
        list = list.push(
            mouse_area(
                container(text(line).size(tea.body()).font(icedtea::typo::UI))
                    .width(Fill)
                    .padding([4, 8])
                    .style(move |_| icedtea::style::list_row(tea, on)),
            )
            .on_press(Message::CommandRun(hit.id)),
        );
    }
    let q = palette.command_query();
    let prompt = if q.is_empty() {
        "type to filter actions".to_string()
    } else {
        format!(": {q}")
    };
    container(
        column![
            widget::label(
                "commands",
                widget::LabelFace::Body,
                tea,
                A11y::new("commands", Role::Header),
            ),
            meta(prompt, tea, A11y::new("query", Role::Status)),
            widget::scroll(
                list.into(),
                tea,
                A11y::new("command-list", Role::List),
                false,
                None,
                None::<fn(f32) -> Message>,
            ),
            meta(
                "enter runs the same handler as the chord",
                tea,
                A11y::new("hint", Role::Status),
            ),
        ]
        .spacing(12)
        .padding(24),
    )
    .width(Fill)
    .height(Fill)
    .style(move |_| icedtea::style::shell(tea))
    .into()
}

fn help_overlay(palette: &Palette, tea: Tokens) -> Element<'_, Message> {
    let doc = palette.help_md();
    container(
        column![
            widget::label(
                "vissue",
                widget::LabelFace::Body,
                tea,
                A11y::new("help", Role::Header),
            ),
            widget::scroll(
                widget::markdown_view(
                    &doc.items,
                    None,
                    |_| Message::Noop,
                    tea,
                    |url| Message::MdLink(url.to_string()),
                    A11y::new("help-md", Role::Group),
                    widget::MarkdownOpts::default(),
                ),
                tea,
                A11y::new("help-scroll", Role::Group),
                false,
                None,
                None::<fn(f32) -> Message>,
            ),
            meta("esc closes help", tea, A11y::new("hint", Role::Status)),
        ]
        .spacing(12)
        .padding(24),
    )
    .width(Fill)
    .height(Fill)
    .style(move |_| icedtea::style::shell(tea))
    .into()
}

fn detail_tab_body<'a>(palette: &'a Palette, tea: Tokens) -> Element<'a, Message> {
    match palette.detail_tab() {
        DetailTab::Tree => tree_list(palette, tea),
        DetailTab::Related => related_list(palette, tea),
        DetailTab::Notes => notes_body(palette, tea),
        DetailTab::Recall => recall_list(palette, tea),
    }
}

fn select_field<'a>(
    palette: &'a Palette,
    id: &str,
    face: FontFace,
    tea: Tokens,
) -> Option<Element<'a, Message>> {
    let content = palette.selectable(id)?;
    let field = id.to_string();
    Some(widget::selectable(
        content,
        {
            let field = field.clone();
            move |action| Message::SelectField(field.clone(), action)
        },
        tea,
        face,
        A11y::new(field, Role::TextBox),
    ))
}

fn tree_fold_all(all_open: bool, tea: Tokens) -> Element<'static, Message> {
    let (label, msg) = if all_open {
        ("Collapse all", Message::TreeCollapseAll)
    } else {
        ("Expand all", Message::TreeExpandAll)
    };
    button(text(label).size(tea.meta()).color(tea.muted))
        .padding([2, 6])
        .style(icedtea::style::button_style(tea, Variant::Ghost))
        .on_press(msg)
        .into()
}

fn tree_list<'a>(palette: &'a Palette, tea: Tokens) -> Element<'a, Message> {
    let rows = palette.tree_rows();
    if rows.is_empty() {
        return meta(
            tab_empty_copy(DetailTab::Tree),
            tea,
            A11y::new("detail-body", Role::Group),
        );
    }
    let selected = palette.tree_selected();
    let mut col = column![tree_fold_all(palette.tree_all_expanded(), tea)].spacing(4);
    for row in rows {
        col = col.push(tree_row(row, selected == Some(row.tea_id), tea));
    }
    col.into()
}

fn notes_body<'a>(palette: &'a Palette, tea: Tokens) -> Element<'a, Message> {
    let Some(detail) = palette.detail() else {
        return meta(
            tab_empty_copy(DetailTab::Notes),
            tea,
            A11y::new("notes", Role::Group),
        );
    };
    if detail.logbook.is_empty() {
        return meta(
            tab_empty_copy(DetailTab::Notes),
            tea,
            A11y::new("notes", Role::Group),
        );
    }
    let mut col = column![].spacing(6);
    for (i, entry) in detail.logbook.iter().enumerate() {
        let mut card = column![].spacing(4).width(Fill);
        if !entry.timestamp.is_empty() {
            card = card.push(meta(
                crate::dates::format_org_stamps(&entry.timestamp),
                tea,
                A11y::new("when", Role::Status),
            ));
        }
        if let (Some(to), from) = (&entry.to_state, &entry.from_state) {
            let mut flip = row![].spacing(6).align_y(Alignment::Center);
            if let Some(from) = from {
                flip = flip.push(badge_state(from, tea));
            }
            flip = flip.push(
                text(state_arrow(tea.direction))
                    .size(tea.meta())
                    .color(tea.muted),
            );
            flip = flip.push(badge_state(to, tea));
            card = card.push(flip);
        }
        if let Some(note) = select_field(palette, &format!("note-{i}"), FontFace::Ui, tea) {
            card = card.push(note);
        }
        if let Some(clock) = select_field(palette, &format!("clock-{i}"), FontFace::Mono, tea) {
            card = card.push(clock);
        }
        col = col.push(
            container(card.padding([6, 4]))
                .width(Fill)
                .style(move |_| icedtea::style::card(tea, false)),
        );
    }
    col.into()
}

fn issue_main<'a>(palette: &'a Palette, tea: Tokens) -> Element<'a, Message> {
    widget::scroll(
        column![issue_fields(palette, tea), issue_prose(palette, tea)]
            .spacing(12)
            .width(Fill)
            .into(),
        tea,
        A11y::new("issue", Role::Group),
        false,
        Some(Id::from(Palette::PREVIEW_SCROLL_ID)),
        Some(Message::PreviewScrolled),
    )
}

fn issue_fields<'a>(palette: &'a Palette, tea: Tokens) -> Element<'a, Message> {
    if palette.excerpt().is_none_or(|excerpt| excerpt.suppressed) || palette.detail().is_none() {
        return column![].into();
    }
    let label_w = palette.excerpt_label_width();
    let mut table = column![].spacing(6).width(Fill);
    for (id, label) in palette.excerpt_form() {
        let Some(content) = palette.selectable(&id) else {
            continue;
        };
        let bind = id.clone();
        table = table.push(widget::value_field(
            label,
            content,
            move |action| Message::SelectField(bind.clone(), action),
            None,
            FontFace::Ui,
            label_w,
            tea,
            tea.direction,
            A11y::new(id, Role::Group),
        ));
    }
    table.into()
}

fn issue_prose<'a>(palette: &'a Palette, tea: Tokens) -> Element<'a, Message> {
    let Some(excerpt) = palette.excerpt() else {
        return meta(
            if palette.detail_body().is_empty() {
                "Select a row."
            } else {
                palette.detail_body()
            },
            tea,
            A11y::new("excerpt", Role::Group),
        );
    };
    if excerpt.suppressed {
        return meta(
            excerpt.text.as_str(),
            tea,
            A11y::new("excerpt", Role::Group),
        );
    }
    let Some(detail) = palette.detail() else {
        return meta(
            if palette.detail_body().is_empty() {
                "Select a row."
            } else {
                palette.detail_body()
            },
            tea,
            A11y::new("excerpt", Role::Group),
        );
    };
    if detail.body.trim().is_empty() {
        return meta("No body.", tea, A11y::new("body", Role::Status));
    }
    select_field(palette, "excerpt-body", FontFace::Ui, tea)
        .unwrap_or_else(|| meta("No body.", tea, A11y::new("body", Role::Status)))
}

fn related_list<'a>(palette: &'a Palette, tea: Tokens) -> Element<'a, Message> {
    let hits = palette.related_hits();
    if hits.is_empty() {
        return meta(
            tab_empty_copy(DetailTab::Related),
            tea,
            A11y::new("related", Role::Group),
        );
    }
    let mut col = column![].spacing(4);
    let selected = palette.painted_id();
    for hit in hits {
        let (priority, blocked, claimed) = palette
            .related_marks(&hit.id)
            .map(|(p, b, c)| (Some(p), b, c))
            .unwrap_or((None, false, false));
        let why = related_why(&hit.evidence);
        let id = hit.id.clone();
        let title_color = if hit.state == "DONE" {
            tea.muted
        } else {
            tea.text
        };
        let marks = row![
            issue_marks(priority, &hit.state, blocked, claimed, tea),
            project_mark(&hit.project, tea),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Fill);
        let mut body = column![
            marks,
            text(hit.title.clone())
                .size(tea.body())
                .font(icedtea::typo::UI)
                .color(title_color)
                .wrapping(iced::widget::text::Wrapping::Word)
                .width(Fill),
        ]
        .spacing(4)
        .width(Fill)
        .align_x(Alignment::Start);
        if !why.is_empty() {
            body = body.push(meta(why, tea, A11y::new("why", Role::Status)));
        }
        let selected = selected == Some(hit.id.as_str());
        col = col.push(
            mouse_area(
                container(body.padding([8, 6]))
                    .width(Fill)
                    .style(move |_| icedtea::style::card(tea, selected)),
            )
            .on_press(Message::OpenIssue(id)),
        );
    }
    col.into()
}

/// The working set: the plan above the issue, then one card per declared
/// input with the deeds it produced.
fn recall_list<'a>(palette: &'a Palette, tea: Tokens) -> Element<'a, Message> {
    let Some(set) = palette.recall() else {
        return meta(
            tab_empty_copy(DetailTab::Recall),
            tea,
            A11y::new("recall", Role::Group),
        );
    };
    if set.plan.is_empty() && set.inputs.is_empty() && set.produced.is_empty() {
        return meta(
            tab_empty_copy(DetailTab::Recall),
            tea,
            A11y::new("recall", Role::Group),
        );
    }
    let mut col = column![].spacing(4);
    for step in &set.plan {
        let id = step.id.clone();
        col = col.push(
            mouse_area(
                container(
                    column![
                        meta(
                            format!("plan  {}", step.id),
                            tea,
                            A11y::new("plan", Role::Status)
                        ),
                        text(step.title.clone())
                            .size(tea.body())
                            .font(icedtea::typo::UI)
                            .color(tea.muted)
                            .wrapping(iced::widget::text::Wrapping::Word)
                            .width(Fill),
                    ]
                    .spacing(4)
                    .padding([8, 6]),
                )
                .width(Fill)
                .style(move |_| icedtea::style::card(tea, false)),
            )
            .on_press(Message::OpenIssue(id)),
        );
    }
    for input in &set.inputs {
        let id = input.id.clone();
        let title_color = if input.state == "DONE" {
            tea.muted
        } else {
            tea.text
        };
        let mut body = column![
            meta(
                format!("{}  {}  [{}]", input.state, input.id, input.relation),
                tea,
                A11y::new("input", Role::Status)
            ),
            text(input.title.clone())
                .size(tea.body())
                .font(icedtea::typo::UI)
                .color(title_color)
                .wrapping(iced::widget::text::Wrapping::Word)
                .width(Fill),
        ]
        .spacing(4)
        .width(Fill)
        .align_x(Alignment::Start);
        for deed in &input.deeds {
            body = body.push(meta(deed.clone(), tea, A11y::new("deed", Role::Status)));
        }
        if input.deeds.is_empty() {
            body = body.push(meta(
                "no deeds cited".to_string(),
                tea,
                A11y::new("deed", Role::Status),
            ));
        }
        if let Some(note) = &input.last_note {
            body = body.push(meta(
                format!("note: {}", note.lines().next().unwrap_or_default().trim()),
                tea,
                A11y::new("note", Role::Status),
            ));
        }
        col = col.push(
            mouse_area(
                container(body.padding([8, 6]))
                    .width(Fill)
                    .style(move |_| icedtea::style::card(tea, false)),
            )
            .on_press(Message::OpenIssue(id)),
        );
    }
    for deed in &set.produced {
        col = col.push(meta(
            format!("produced  {deed}"),
            tea,
            A11y::new("produced", Role::Status),
        ));
    }
    col.into()
}

fn related_why(evidence: &[String]) -> String {
    evidence
        .iter()
        .filter_map(|reason| {
            if reason.starts_with("score") {
                return None;
            }
            if reason == "blocked_by" || reason.starts_with("blocked") {
                return Some("blocked".to_string());
            }
            if reason.starts_with("term:") {
                return None;
            }
            if let Some(n) = reason.strip_prefix("org_distance:") {
                return Some(if n == "1" {
                    "nearby".to_string()
                } else {
                    format!("distance {n}")
                });
            }
            Some(reason.clone())
        })
        .collect::<Vec<_>>()
        .join("  ·  ")
}

fn state_arrow(dir: Direction) -> &'static str {
    match dir {
        Direction::Ltr => "→",
        Direction::Rtl => "←",
    }
}

fn tab_empty_copy(tab: DetailTab) -> &'static str {
    match tab {
        DetailTab::Tree => "No parent or child links.",
        DetailTab::Related => "No related issues.",
        DetailTab::Notes => "No logbook yet. n writes a note.",
        DetailTab::Recall => "Nothing declared: this stands on nothing.",
    }
}

fn pane_scroll<'a>(child: Element<'a, Message>, tea: Tokens, a11y: A11y) -> Element<'a, Message> {
    widget::scroll(child, tea, a11y, false, None, None::<fn(f32) -> Message>)
}

/// icedtea 0.17 folded `widget::meta` into [`widget::label`] + [`widget::LabelFace::Meta`].
fn meta<'a>(s: impl Into<String>, tea: Tokens, a11y: A11y) -> Element<'a, Message> {
    widget::label(s, widget::LabelFace::Meta, tea, a11y)
}

fn empty_copy(palette: &Palette) -> &'static str {
    if palette.filter() == BoardFilter::Search && palette.query().is_empty() {
        return "Type to search id, title, and tags.";
    }
    if !palette.query().is_empty() {
        return "Nothing matches.";
    }
    match palette.filter() {
        BoardFilter::Ready => "Nothing ready in this project.",
        BoardFilter::List => "No issues in this project.",
        BoardFilter::Claims => "No claims.",
        BoardFilter::Agenda => "Nothing dated in the next two weeks.",
        BoardFilter::Search => "Type to search id, title, and tags.",
    }
}

#[cfg(test)]
mod tests {
    use crate::palette::DetailTab;

    /// The body of one function in this file; there is no widget tree to walk
    /// headless.
    fn between<'a>(src: &'a str, from: &str, to: &str) -> &'a str {
        let after = src
            .split(from)
            .nth(1)
            .unwrap_or_else(|| panic!("no {from} in view.rs"));
        after
            .split(to)
            .next()
            .unwrap_or_else(|| panic!("no {to} after {from}"))
    }

    /// The board is painted out of icedtea's widgets, reported as one list.
    #[test]
    fn the_board_paints_through_icedtea_widgets() {
        let src = include_str!("view.rs");
        const WIDGETS: &[&str] = &[
            "widget::chip",
            "widget::themed_text_input",
            "widget::search_input_clear",
            "widget::list_view",
            "widget::virtual_column",
            "widget::themed_checkbox",
            "split_view",
            "widget::themed_scroll",
            "icedtea::style::list_row",
            "widget::tab_bar",
            "widget::selectable",
            "widget::value_field",
        ];
        let missing: Vec<&str> = WIDGETS
            .iter()
            .copied()
            .filter(|widget| !src.contains(widget))
            .collect();
        assert!(
            missing.is_empty(),
            "the view no longer paints with: {missing:?}"
        );
        assert!(
            src.contains("icedtea::widget::markdown_view") || src.contains("widget::markdown_view"),
            "notes are not painted through the markdown view"
        );

        const FUNCTIONS: &[&str] = &[
            "fn tree_row",
            "fn tree_fold_all",
            "fn extra_blocked_mark",
            "fn notes_body",
            "fn state_arrow",
        ];
        let gone: Vec<&str> = FUNCTIONS
            .iter()
            .copied()
            .filter(|name| !src.contains(name))
            .collect();
        assert!(gone.is_empty(), "the view no longer defines: {gone:?}");
    }

    /// No text in a list row clips; asserted as the absence of the clipping call.
    #[test]
    fn a_list_row_wraps_its_text_rather_than_clipping_it() {
        let src = include_str!("view.rs");
        let task = between(src, "fn task_row", "fn extra_blocked_mark");
        assert!(task.contains("Wrapping::Word"), "list titles must wrap");
        assert!(
            !task.contains("Wrapping::None"),
            "a text in a list row is set not to wrap, so it will clip"
        );
    }

    /// A tree row answers three gestures: pick it, open it, and read its state.
    #[test]
    fn a_tree_row_picks_opens_and_shows_its_state() {
        let src = include_str!("view.rs");
        let tree = between(src, "fn tree_row", "fn task_row");
        for expected in [
            "Message::TreePick",
            "on_double_click",
            "Message::OpenIssue",
            "badge_state",
        ] {
            assert!(tree.contains(expected), "a tree row has no {expected}");
        }
    }

    /// A project is marked with a chip, which is a label, and never with the danger
    /// variant, which is a warning about the project rather than a name for it.
    #[test]
    fn a_project_mark_is_a_label_and_not_a_warning() {
        let src = include_str!("view.rs");
        let mark = between(src, "fn project_mark", "fn badge_priority");
        assert!(mark.contains("Variant::Chip"));
        assert!(!mark.contains("Variant::Danger"));
    }

    #[test]
    fn tree_indent_is_wider_than_the_twisty() {
        let tea = crate::theme::tokens();
        let step = crate::view::tree_indent_px(1, tea);
        let twisty = tea.density.gap() * 2.0 + 4.0;
        assert!(
            step > twisty,
            "one hop {step} must read deeper than twisty {twisty}"
        );
        assert_eq!(crate::view::tree_indent_px(3, tea), step * 3.0);
    }

    #[test]
    fn extra_blocked_mark_does_not_repeat_state() {
        assert!(!crate::view::extra_blocked_mark("BLOCKED", true));
        assert!(!crate::view::extra_blocked_mark("CANCELLED", true));
        assert!(!crate::view::extra_blocked_mark("DONE", true));
        assert!(crate::view::extra_blocked_mark("TODO", true));
        assert!(crate::view::extra_blocked_mark("STARTED", true));
        assert!(!crate::view::extra_blocked_mark("TODO", false));
    }

    /// The narrow overlay fits three tab labels (48px of chrome per tab in
    /// 360px), the wide pane fits every one, and the keyboard reaches every tab.
    #[test]
    fn every_side_tab_is_reachable_even_where_the_strip_clips() {
        let titles: Vec<String> = DetailTab::ALL
            .iter()
            .map(|t| t.label().to_string())
            .collect();
        assert_eq!(icedtea::widget::tab_visible_count(&titles, 360.0), 3);
        assert_eq!(
            icedtea::widget::tab_visible_count(&titles, 960.0),
            DetailTab::ALL.len(),
            "the wide pane has room for all of them and must show all of them"
        );

        let mut seen = vec![DetailTab::ALL[0]];
        let mut at = DetailTab::ALL[0];
        for _ in 1..DetailTab::ALL.len() {
            at = at.next();
            assert!(!seen.contains(&at), "the cycle repeats before it finishes");
            seen.push(at);
        }
        assert_eq!(
            at.next(),
            DetailTab::ALL[0],
            "the cycle has to close, or the last tab is a dead end"
        );
        for tab in DetailTab::ALL {
            assert!(seen.contains(&tab), "{tab:?} is not reachable by cycling");
        }
    }
}
