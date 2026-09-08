//! iced chrome around [`crate::palette::Palette`].

use std::path::PathBuf;
use std::time::Duration;

use iced::event::{self, Event};
use iced::keyboard::{self, Key, key::Named};
use iced::window;
use iced::{Element, Font, Pixels, Subscription, Task, time};

use vissue_core::config::Layout;

use crate::attach;
use crate::palette::{BoardFilter, DetailTab, Palette, PaletteKey};
use crate::summon;
use crate::theme;

const HUD_W: f32 = 960.0;
const HUD_H: f32 = 760.0;

/// First-paint inputs for the board.
#[derive(Clone, Debug)]
pub struct BootOpts {
    /// Vault root and project prefix.
    pub layout: Layout,
    /// Control socket to attach after first paint.
    pub socket: PathBuf,
    /// Skip the socket and stay on [`vissue_tui::CoreBackend`].
    pub offline: bool,
    /// Identity stamped on claims and updates.
    pub agent: String,
    /// Whether the window starts mapped.
    pub visible: bool,
}

/// iced messages. Mapping from native keys lives here so view stays dumb.
#[derive(Debug, Clone)]
pub enum Message {
    /// Board key after native mapping.
    Key(PaletteKey),
    /// 50 ms poll: serve wait plus summon socket.
    Tick,
    /// Latest iced window id, once the shell reports one.
    WindowId(Option<window::Id>),
    /// Window close request: leave the process while mapped.
    Close,
    /// Compositor deleted the surface: exit if mapped, clear if hidden.
    Closed,
    /// Switch the list filter chip.
    Filter(BoardFilter),
    /// Select the row with this issue id.
    SelectId(String),
    /// Toggle DONE on this issue id.
    ToggleDone(String),
    /// Replace the search query.
    QueryChanged(String),
    /// Replace the add-task draft.
    AddChanged(String),
    /// Submit the add-task draft.
    AddSubmit,
    /// Replace the logbook note draft.
    NoteChanged(String),
    /// The deed accession field changed.
    DeedChanged(String),
    /// Submit the logbook note draft.
    NoteSubmit,
    /// Cite the deed accession in the field.
    DeedSubmit,
    /// Focus the add-task field.
    FocusAdd,
    /// Return typing to the row list.
    FocusList,
    /// Open this detail tab.
    DetailTab(DetailTab),
    /// Collapse or expand this project group.
    ToggleProject(String),
    /// Enter this project from the home list.
    SelectProject(String),
    /// Enter the home-list card at this index.
    PickProject(usize),
    /// Home project list scroll window.
    ProjectScroll(icedtea::collection::VisibleWindow),
    /// Task board scroll window.
    TaskScroll(icedtea::collection::VisibleWindow),
    /// Return to the home project list.
    LeaveProject,
    /// Copy a markdown link target.
    MdLink(String),
    /// Expand or collapse a Tree-tab node.
    TreeToggle(u64),
    /// Expand every Tree-tab node that has children.
    TreeExpandAll,
    /// Collapse every Tree-tab node that has children.
    TreeCollapseAll,
    /// Highlight the issue under a Tree-tab node.
    TreePick(u64),
    /// Open this issue as the selected board row.
    OpenIssue(String),
    /// Drag-select inside a read-only detail field.
    SelectField(String, iced::widget::text_editor::Action),
    /// Drag the list/detail sash.
    Sash(icedtea::layout::SashEvent),
    /// Window height changed.
    WindowResized(f32),
    /// Discarded click (tab-bar close, unused).
    Noop,
}

/// iced application state.
#[derive(Debug)]
pub struct HudApp {
    /// Overlay state the view reads.
    pub palette: Palette,
    window_id: Option<window::Id>,
    /// Reserved id for an in-flight `window::open`. Late completions
    /// after hide must not restore a closed surface.
    opening: Option<window::Id>,
    /// Remaining Sway IPC attempts after a show. Zero when placed or
    /// Sway is absent.
    place_tries: u8,
}

impl HudApp {
    fn from_palette(palette: Palette) -> Self {
        Self {
            palette,
            window_id: None,
            opening: None,
            place_tries: 0,
        }
    }

    /// Compositor close leaves the process only while the overlay is mapped.
    fn close_exits(&self) -> bool {
        self.palette.visible()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowId(id) => {
                if id != self.opening {
                    return Task::none();
                }
                self.opening = None;
                self.window_id = id;
                if self.palette.visible() {
                    self.place_tries = 20;
                    self.try_place();
                }
                Task::none()
            }
            Message::Tick => {
                self.palette.poll_updates();
                self.try_place();
                if let Some(req) = summon::try_recv() {
                    let was = self.palette.visible();
                    self.palette.apply_summon(&req);
                    if was != self.palette.visible() {
                        return self.sync_window();
                    }
                }
                Task::none()
            }
            Message::Close => {
                if self.close_exits() {
                    iced::exit()
                } else {
                    Task::none()
                }
            }
            Message::Closed => {
                self.opening = None;
                if self.close_exits() {
                    iced::exit()
                } else {
                    self.window_id = None;
                    Task::none()
                }
            }
            Message::Filter(filter) => {
                self.palette.set_filter(filter);
                Task::none()
            }
            Message::SelectId(id) => {
                self.palette.select_id(&id);
                Task::none()
            }
            Message::ToggleDone(id) => {
                self.palette.toggle_done(&id);
                Task::none()
            }
            Message::QueryChanged(q) => {
                self.palette.type_query(q);
                Task::none()
            }
            Message::AddChanged(text) => {
                self.palette.set_add_draft(text);
                Task::none()
            }
            Message::AddSubmit => {
                self.palette.submit_add();
                Task::none()
            }
            Message::NoteChanged(text) => {
                self.palette.set_note_draft(text);
                Task::none()
            }
            Message::NoteSubmit => {
                self.palette.submit_note();
                Task::none()
            }
            Message::DeedChanged(text) => {
                self.palette.set_deed_draft(text);
                Task::none()
            }
            Message::DeedSubmit => {
                self.palette.submit_deed();
                Task::none()
            }
            Message::FocusAdd => {
                self.palette.focus_add();
                Task::none()
            }
            Message::FocusList => {
                self.palette.focus_list();
                Task::none()
            }
            Message::DetailTab(tab) => {
                self.palette.set_detail_tab(tab);
                Task::none()
            }
            Message::ToggleProject(project) => {
                self.palette.toggle_project(&project);
                Task::none()
            }
            Message::SelectProject(project) => {
                self.palette.enter_project(&project);
                Task::none()
            }
            Message::PickProject(index) => {
                if let Some(name) = self
                    .palette
                    .project_cards()
                    .get(index)
                    .map(|card| card.name.clone())
                {
                    self.palette.enter_project(&name);
                }
                Task::none()
            }
            Message::ProjectScroll(window) => {
                self.palette.set_project_window(window);
                Task::none()
            }
            Message::TaskScroll(window) => {
                self.palette.set_task_window(window);
                Task::none()
            }
            Message::LeaveProject => {
                self.palette.leave_project();
                Task::none()
            }
            Message::MdLink(url) => {
                self.palette.set_clipboard(url);
                Task::none()
            }
            Message::TreeToggle(id) => {
                self.palette.toggle_tree_node(id);
                Task::none()
            }
            Message::TreeExpandAll => {
                self.palette.set_tree_expanded(true);
                Task::none()
            }
            Message::TreeCollapseAll => {
                self.palette.set_tree_expanded(false);
                Task::none()
            }
            Message::TreePick(id) => {
                self.palette.select_tree_node(id);
                Task::none()
            }
            Message::OpenIssue(id) => {
                self.palette.open_issue(&id);
                Task::none()
            }
            Message::SelectField(id, action) => {
                self.palette.perform_select(&id, action);
                Task::none()
            }
            Message::Sash(event) => {
                self.palette.apply_sash(event);
                Task::none()
            }
            Message::WindowResized(height) => {
                self.palette.set_window_height(height);
                Task::none()
            }
            Message::Noop => Task::none(),
            Message::Key(key) => {
                let was = self.palette.visible();
                let before = self.palette.clipboard().to_string();
                self.palette.handle_key(key);
                let clip = self.palette.clipboard().to_string();
                let mut tasks = Vec::new();
                if clip != before && !clip.is_empty() {
                    tasks.push(iced::clipboard::write(clip));
                }
                if was != self.palette.visible() {
                    tasks.push(self.sync_window());
                }
                Task::batch(tasks)
            }
        }
    }

    fn sync_window(&mut self) -> Task<Message> {
        match overlay_action(self.palette.visible(), self.mapped()) {
            OverlayAction::Open => self.open_window(),
            OverlayAction::Close => {
                self.place_tries = 0;
                self.opening = None;
                match self.window_id.take() {
                    Some(id) => window::close(id),
                    None => Task::none(),
                }
            }
            OverlayAction::Place => {
                self.place_tries = 20;
                self.try_place();
                Task::none()
            }
            OverlayAction::Idle => Task::none(),
        }
    }

    fn open_window(&mut self) -> Task<Message> {
        self.place_tries = 20;
        let (id, open) = window::open(board_window());
        self.window_id = Some(id);
        self.opening = Some(id);
        open.map(|id| Message::WindowId(Some(id)))
    }

    fn mapped(&self) -> bool {
        self.window_id.is_some()
    }

    fn try_place(&mut self) {
        if self.place_tries == 0 {
            return;
        }
        if crate::place::place_overlay() || !crate::place::sway_available() {
            self.place_tries = 0;
        } else {
            self.place_tries = self.place_tries.saturating_sub(1);
        }
    }
}

/// Undecorated always-on-top overlay. Sway is told to float it over IPC.
pub fn board_window() -> window::Settings {
    let boot = icedtea::app::Boot::new("vissue", crate::place::OVERLAY_APP_ID)
        .overlay()
        .size(HUD_W, HUD_H)
        .min_size(360.0, 420.0);
    icedtea::app::bootstrap(&boot).window
}

/// First paint via core, attach unless `--offline`, then the iced loop.
///
/// # Errors
///
/// Returns an error if the vault cannot be opened, attach reload fails, or
/// the iced application cannot start.
pub fn run(opts: BootOpts) -> anyhow::Result<()> {
    let mut palette = Palette::open_core(opts.layout, opts.agent)?;
    palette.attach(&opts.socket, opts.offline, &attach::hud_hooks())?;
    if opts.visible {
        palette.show();
    } else {
        palette.hide();
    }
    crate::log::info(&format!(
        "hud start log={} status={}",
        crate::log::path().display(),
        palette.status_line()
    ));
    run_iced(palette).map_err(|err| anyhow::anyhow!("{err}"))
}

fn run_iced(palette: Palette) -> iced::Result {
    icedtea::typo::install_platform_faces();
    let cell = std::sync::Mutex::new(Some(palette));
    iced::daemon(
        move || {
            let palette = cell.lock().expect("boot").take().expect("boot once");
            boot(palette)
        },
        update,
        view,
    )
    .subscription(subscription)
    .theme(|_: &HudApp, _| theme::theme())
    .title(|_: &HudApp, _| "vissue".to_string())
    .default_font(icedtea::typo::UI)
    .settings(iced::Settings {
        default_text_size: Pixels::from(icedtea::typo::BODY),
        default_font: icedtea::typo::UI,
        ..Default::default()
    })
    .run()
}

fn boot(palette: Palette) -> (HudApp, Task<Message>) {
    let visible = palette.visible();
    let mut app = HudApp::from_palette(palette);
    let task = if visible {
        app.open_window()
    } else {
        Task::none()
    };
    (app, task)
}

fn update(app: &mut HudApp, message: Message) -> Task<Message> {
    app.update(message)
}

fn view(app: &HudApp, _id: window::Id) -> Element<'_, Message> {
    crate::view::view(&app.palette)
}

/// What the overlay window should do for a visibility change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverlayAction {
    Open,
    Close,
    Place,
    Idle,
}

fn overlay_action(visible: bool, mapped: bool) -> OverlayAction {
    match (visible, mapped) {
        (true, false) => OverlayAction::Open,
        (false, true) => OverlayAction::Close,
        (true, true) => OverlayAction::Place,
        (false, false) => OverlayAction::Idle,
    }
}

fn subscription(_app: &HudApp) -> Subscription<Message> {
    Subscription::batch([
        event::listen_with(|event, status, _id| match event {
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
                if status == event::Status::Captured && !is_nav_key(&key) {
                    return None;
                }
                map_key(key)
            }
            Event::Window(window::Event::CloseRequested) => Some(Message::Close),
            Event::Window(window::Event::Closed) => Some(Message::Closed),
            _ => None,
        }),
        time::every(Duration::from_millis(50)).map(|_| Message::Tick),
        icedtea::layout::listen_sash()
            .map(|drive| Message::Sash(drive.into_event(icedtea::layout::Axis::Vertical))),
        iced::window::resize_events().map(|(_, size)| Message::WindowResized(size.height)),
    ])
}

fn is_nav_key(key: &Key) -> bool {
    matches!(
        key,
        Key::Named(Named::Escape | Named::Enter | Named::ArrowUp | Named::ArrowDown | Named::Tab)
    )
}

fn map_key(key: Key) -> Option<Message> {
    match key.as_ref() {
        Key::Named(Named::Enter) => Some(Message::Key(PaletteKey::Enter)),
        Key::Named(Named::Escape) => Some(Message::Key(PaletteKey::Esc)),
        Key::Named(Named::ArrowUp) => Some(Message::Key(PaletteKey::Up)),
        Key::Named(Named::ArrowDown) => Some(Message::Key(PaletteKey::Down)),
        Key::Named(Named::Backspace) => Some(Message::Key(PaletteKey::Backspace)),
        Key::Named(Named::Space) => Some(Message::Key(PaletteKey::Space)),
        Key::Named(Named::Tab) => Some(Message::Key(PaletteKey::Tab)),
        Key::Character(c) => {
            let mut chars = c.chars();
            let first = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Some(Message::Key(PaletteKey::Char(first)))
        }
        _ => None,
    }
}

// Keep Font in scope so a missing FACE constant fails here, not in view.
const _: Font = theme::FACE;

#[cfg(test)]
mod tests {
    use super::*;
    use vissue_core::config::{DEFAULT_PREFIX, Layout};

    fn empty_app() -> (tempfile::TempDir, HudApp) {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        std::fs::create_dir_all(layout.projects_dir()).expect("projects dir");
        let mut palette = Palette::open_core(layout, "close".into()).expect("open");
        palette.show();
        (dir, HudApp::from_palette(palette))
    }

    #[test]
    fn a_hidden_overlay_does_not_quit_on_window_close() {
        let (_dir, mut app) = empty_app();
        assert!(app.close_exits());
        app.palette.hide();
        assert!(!app.close_exits());
    }

    #[test]
    fn hide_closes_the_overlay_window() {
        assert_eq!(overlay_action(false, true), OverlayAction::Close);
        assert_eq!(overlay_action(true, false), OverlayAction::Open);
        assert_eq!(overlay_action(true, true), OverlayAction::Place);
        assert_eq!(overlay_action(false, false), OverlayAction::Idle);
        let src = include_str!("app.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(prod.contains("window::close"));
        assert!(prod.contains("iced::daemon"));
        assert!(prod.contains("window::Event::Closed"));
        assert!(!prod.contains("Mode::Hidden"));
        assert!(!prod.contains("set_mode"));
    }

    #[test]
    fn a_stale_window_id_after_hide_does_not_count_as_mapped() {
        let (_dir, mut app) = empty_app();
        let id = window::Id::unique();
        app.opening = Some(id);
        let _ = app.update(Message::WindowId(Some(id)));
        assert!(app.mapped());
        app.palette.hide();
        let _ = app.sync_window();
        assert!(!app.mapped());
        let _ = app.update(Message::WindowId(Some(id)));
        assert!(
            !app.mapped(),
            "a late open completion must not restore a closed surface"
        );
        app.palette.show();
        assert_eq!(
            overlay_action(app.palette.visible(), app.mapped()),
            OverlayAction::Open
        );
    }

    #[test]
    fn closed_while_hidden_clears_mapped_state() {
        let (_dir, mut app) = empty_app();
        let id = window::Id::unique();
        app.opening = Some(id);
        let _ = app.update(Message::WindowId(Some(id)));
        assert!(app.mapped());
        app.palette.hide();
        assert!(!app.close_exits());
        let _ = app.update(Message::Closed);
        assert!(!app.mapped());
        app.palette.show();
        assert_eq!(
            overlay_action(app.palette.visible(), app.mapped()),
            OverlayAction::Open
        );
    }
}
