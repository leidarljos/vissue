//! iced widgets for the task board. Drawing only; logic lives in [`crate::palette`].

use iced::widget::{button, checkbox, column, container, row, scrollable, text, text_input, Space};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length};

use crate::app::Message;
use crate::palette::{BoardFilter, DetailTab, Focus, HudItem, Palette};
use crate::theme;

/// Board face. Hidden state draws nothing so a `Mode::Hidden` frame is empty.
pub fn view(palette: &Palette) -> Element<'_, Message> {
    if !palette.visible() {
        return Space::new().width(0).height(0).into();
    }
    if palette.focus() == Focus::Help {
        return help_overlay(palette);
    }

    let mut pane = column![header(palette), chips(palette), add_bar(palette)]
        .spacing(10)
        .padding([14, 16]);

    let items = palette.filtered_items();
    if items.is_empty() {
        pane = pane.push(
            text(empty_copy(palette))
                .size(theme::SIZE_BODY)
                .color(theme::SUBTEXT)
                .font(theme::FACE),
        );
    } else {
        let mut list = column![].spacing(2);
        for (i, item) in items.into_iter().enumerate() {
            list = list.push(task_row(item, i == palette.selected_index()));
        }
        pane = pane.push(scrollable(list).height(Length::FillPortion(3)));
    }

    pane = pane.push(detail_panel(palette));

    if palette.note_draft().is_some() && palette.detail_tab() != DetailTab::Notes {
        pane = pane.push(note_bar(palette));
    }
    if let Some(kind) = palette.confirm() {
        pane = pane.push(
            text(format!("confirm {}? y/n", kind.state()))
                .size(theme::SIZE_BODY)
                .color(theme::PEACH)
                .font(theme::FACE),
        );
    }
    if !palette.message().is_empty() {
        pane = pane.push(
            text(palette.message())
                .size(theme::SIZE_META)
                .color(theme::PEACH)
                .font(theme::FACE),
        );
    }

    container(pane.spacing(10).width(Fill).height(Fill))
        .width(Fill)
        .height(Fill)
        .style(|_| fill(theme::BASE))
        .into()
}

fn header(palette: &Palette) -> Element<'_, Message> {
    let live = match palette.serve_status() {
        vissue_tui::attach::ServeStatus::Live => theme::GREEN,
        vissue_tui::attach::ServeStatus::Mismatch => theme::PEACH,
        vissue_tui::attach::ServeStatus::Offline => theme::OVERLAY,
    };
    let dot = container(Space::new().width(8).height(8)).style(move |_| container::Style {
        background: Some(Background::Color(live)),
        border: Border {
            radius: 4.0.into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        ..container::Style::default()
    });
    let project = palette.project().unwrap_or("all projects");
    let title = row![
        text("vissue")
            .size(theme::SIZE_TITLE)
            .color(theme::TEXT)
            .font(theme::FACE),
        dot,
        text(project)
            .size(theme::SIZE_META)
            .color(theme::SUBTEXT)
            .font(theme::FACE),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    row![title, Space::new().width(Fill), find_control(palette)]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
}

fn find_control(palette: &Palette) -> Element<'_, Message> {
    if palette.focus() == Focus::Search || palette.filter() == BoardFilter::Search {
        text_input("Search title, id, tags", palette.query())
            .on_input(Message::QueryChanged)
            .on_submit(Message::FocusList)
            .size(theme::SIZE_META)
            .padding([8, 10])
            .font(theme::FACE)
            .width(260)
            .style(|_, _| input_style())
            .into()
    } else {
        button(
            text("Search")
                .size(theme::SIZE_META)
                .color(theme::SUBTEXT)
                .font(theme::FACE),
        )
        .on_press(Message::FocusSearch)
        .padding([8, 12])
        .style(|_, status| chip_style(status, false))
        .into()
    }
}

fn chips(palette: &Palette) -> Element<'_, Message> {
    let mut row = row![].spacing(6);
    for (filter, label) in BoardFilter::ALL {
        row = row.push(chip(
            filter,
            label,
            palette.count(filter),
            palette.filter() == filter,
        ));
    }
    row.into()
}

fn chip(
    filter: BoardFilter,
    label: &'static str,
    count: usize,
    active: bool,
) -> Element<'static, Message> {
    let fg = if active { theme::BASE } else { theme::SUBTEXT };
    button(
        text(format!("{label} {count}"))
            .size(theme::SIZE_META)
            .color(fg)
            .font(theme::FACE),
    )
    .on_press(Message::Filter(filter))
    .padding([6, 12])
    .style(move |_, status| {
        let hovered = matches!(status, button::Status::Hovered);
        button::Style {
            background: Some(Background::Color(if active {
                theme::BLUE
            } else if hovered {
                theme::SURFACE1
            } else {
                theme::SURFACE0
            })),
            text_color: fg,
            border: Border {
                radius: 14.0.into(),
                width: 0.0,
                color: Color::TRANSPARENT,
            },
            ..button::Style::default()
        }
    })
    .into()
}

fn add_bar(palette: &Palette) -> Element<'_, Message> {
    if palette.focus() == Focus::Add {
        text_input("Add a task in the current project", palette.add_draft())
            .on_input(Message::AddChanged)
            .on_submit(Message::AddSubmit)
            .size(theme::SIZE_BODY)
            .padding(12)
            .font(theme::FACE)
            .style(|_, _| input_style())
            .into()
    } else {
        button(
            row![
                text("+")
                    .size(theme::SIZE_TITLE)
                    .color(theme::BLUE)
                    .font(theme::FACE),
                text("Add a task")
                    .size(theme::SIZE_BODY)
                    .color(theme::OVERLAY)
                    .font(theme::FACE),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .on_press(Message::FocusAdd)
        .padding([10, 12])
        .width(Fill)
        .style(|_, status| {
            let hovered = matches!(status, button::Status::Hovered);
            button::Style {
                background: Some(Background::Color(if hovered {
                    theme::SURFACE1
                } else {
                    theme::SURFACE0
                })),
                text_color: theme::OVERLAY,
                border: Border {
                    radius: 12.0.into(),
                    width: 1.0,
                    color: if hovered {
                        theme::BLUE
                    } else {
                        theme::SURFACE1
                    },
                },
                ..button::Style::default()
            }
        })
        .into()
    }
}

fn task_row(item: &HudItem, selected: bool) -> Element<'_, Message> {
    let done = item.state == "DONE";
    let title_color = if done { theme::OVERLAY } else { theme::TEXT };
    let id = item.id.clone();
    let pip_color = theme::priority_color(&item.priority);
    let mut meta = format!(
        "{}  [{}]  [#{}]  {}",
        item.id, item.state, item.priority, item.project
    );
    if !item.blocked_by.is_empty() {
        meta = format!("{meta}  blocked by {}", item.blocked_by.join(","));
    }
    if !item.extra.is_empty() {
        meta = format!("{meta}  {}", item.extra);
    }

    let pip = container(Space::new().width(4).height(34)).style(move |_| container::Style {
        background: Some(Background::Color(pip_color)),
        border: Border {
            radius: 2.0.into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        ..container::Style::default()
    });

    let titles = column![
        text(item.title.clone())
            .size(theme::SIZE_TITLE)
            .color(title_color)
            .font(theme::FACE),
        text(meta)
            .size(theme::SIZE_META)
            .color(theme::state_color(&item.state))
            .font(theme::FACE),
    ]
    .spacing(2);

    let body = row![
        checkbox(done)
            .on_toggle(move |_| Message::ToggleDone(id.clone()))
            .size(18)
            .style(move |_, status| check_style(status, done)),
        pip,
        titles,
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .padding([8, 6]);

    let idx = item.id.clone();
    button(body)
        .on_press(Message::SelectId(idx))
        .padding(0)
        .width(Fill)
        .style(move |_, status| {
            let hovered = matches!(status, button::Status::Hovered);
            button::Style {
                background: Some(Background::Color(if selected {
                    theme::SURFACE0
                } else if hovered {
                    theme::MANTLE
                } else {
                    Color::TRANSPARENT
                })),
                text_color: title_color,
                border: Border {
                    radius: 10.0.into(),
                    width: if selected { 1.0 } else { 0.0 },
                    color: if selected {
                        theme::SURFACE1
                    } else {
                        Color::TRANSPARENT
                    },
                },
                ..button::Style::default()
            }
        })
        .into()
}

fn detail_panel(palette: &Palette) -> Element<'_, Message> {
    let mut tabs = row![].spacing(6);
    for tab in DetailTab::ALL {
        let active = palette.detail_tab() == tab;
        tabs = tabs.push(
            button(
                text(tab.label())
                    .size(theme::SIZE_META)
                    .color(if active { theme::BASE } else { theme::SUBTEXT })
                    .font(theme::FACE),
            )
            .on_press(Message::DetailTab(tab))
            .padding([4, 10])
            .style(move |_, status| chip_style(status, active)),
        );
    }

    let body = if palette.detail_body().is_empty() {
        "Select a row."
    } else {
        palette.detail_body()
    };

    let mut card = column![
        tabs,
        scrollable(
            text(body)
                .size(theme::SIZE_META)
                .color(theme::TEXT)
                .font(theme::FACE)
        )
        .height(Length::Fill),
    ]
    .spacing(8)
    .padding(12);
    if palette.detail_tab() == DetailTab::Notes {
        card = card.push(note_bar(palette));
    }

    container(card)
        .width(Fill)
        .height(Length::FillPortion(2))
        .style(|_| container::Style {
            background: Some(Background::Color(theme::MANTLE)),
            border: Border {
                radius: 12.0.into(),
                width: 1.0,
                color: theme::SURFACE1,
            },
            ..container::Style::default()
        })
        .into()
}

fn note_bar(palette: &Palette) -> Element<'_, Message> {
    let draft = palette.note_draft().unwrap_or("");
    text_input("Add a note to the logbook", draft)
        .on_input(Message::NoteChanged)
        .on_submit(Message::NoteSubmit)
        .size(theme::SIZE_BODY)
        .padding(10)
        .font(theme::FACE)
        .style(|_, _| input_style())
        .into()
}

fn help_overlay(palette: &Palette) -> Element<'_, Message> {
    container(
        column![
            text("vissue")
                .size(theme::SIZE_TITLE)
                .color(theme::TEXT)
                .font(theme::FACE),
            scrollable(
                text(palette.help_text())
                    .size(theme::SIZE_META)
                    .color(theme::SUBTEXT)
                    .font(theme::FACE)
            ),
            text("esc closes help")
                .size(theme::SIZE_HINT)
                .color(theme::OVERLAY)
                .font(theme::FACE),
        ]
        .spacing(12)
        .padding(24),
    )
    .width(Fill)
    .height(Fill)
    .style(|_| fill(theme::BASE))
    .into()
}

fn empty_copy(palette: &Palette) -> &'static str {
    if palette.filter() == BoardFilter::Search && palette.query().is_empty() {
        return "Type to search id, title, and tags.";
    }
    if !palette.query().is_empty() {
        return "Nothing matches.";
    }
    match palette.filter() {
        BoardFilter::Ready => "Nothing ready.",
        BoardFilter::List => "No issues in this filter.",
        BoardFilter::Claims => "No claims.",
        BoardFilter::Agenda => "Nothing dated in the next two weeks.",
        BoardFilter::Search => "Type to search id, title, and tags.",
    }
}

fn fill(color: Color) -> container::Style {
    container::Style {
        background: Some(Background::Color(color)),
        text_color: Some(theme::TEXT),
        ..container::Style::default()
    }
}

fn input_style() -> text_input::Style {
    text_input::Style {
        background: Background::Color(theme::SURFACE0),
        border: Border {
            radius: 12.0.into(),
            width: 1.0,
            color: theme::SURFACE1,
        },
        icon: theme::SUBTEXT,
        placeholder: theme::OVERLAY,
        value: theme::TEXT,
        selection: theme::BLUE,
    }
}

fn chip_style(status: button::Status, active: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered);
    button::Style {
        background: Some(Background::Color(if active {
            theme::BLUE
        } else if hovered {
            theme::SURFACE1
        } else {
            theme::SURFACE0
        })),
        text_color: if active { theme::BASE } else { theme::SUBTEXT },
        border: Border {
            radius: 12.0.into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        ..button::Style::default()
    }
}

fn check_style(status: checkbox::Status, done: bool) -> checkbox::Style {
    let accent = if done { theme::GREEN } else { theme::SURFACE1 };
    let hovered = matches!(status, checkbox::Status::Hovered { .. });
    checkbox::Style {
        background: Background::Color(if done {
            theme::GREEN
        } else if hovered {
            theme::SURFACE1
        } else {
            theme::MANTLE
        }),
        icon_color: theme::BASE,
        border: Border {
            radius: 6.0.into(),
            width: 1.5,
            color: accent,
        },
        text_color: Some(theme::TEXT),
    }
}
