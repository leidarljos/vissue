//! Draw the board. No catalog or mutation logic.

use std::io::{self, Stdout, stdout};

use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs, Wrap};
use ratatui::{Frame, Terminal};

use crate::app::App;
use crate::keys::Pane;

/// Crossterm terminal used by [`install`].
pub type CrosstermTerm = Terminal<CrosstermBackend<Stdout>>;

/// Enter raw mode and the alternate screen.
///
/// # Errors
///
/// Returns an error if the terminal cannot enter raw mode, cannot switch to
/// the alternate screen, or cannot be wrapped as a ratatui backend.
pub fn install() -> io::Result<CrosstermTerm> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(out))
}

/// Leave raw mode and the alternate screen.
///
/// # Errors
///
/// Returns an error if the terminal cannot leave raw mode or the alternate
/// screen.
pub fn restore() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Draw tabs, list, detail, status, and any prompt or help overlay.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let [tabs, body, status] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_tabs(frame, tabs, app);
    let [list, detail] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(body);
    draw_list(frame, list, app);
    draw_detail(frame, detail, app);
    draw_status(frame, status, app);

    if app.help {
        draw_overlay(frame, area, &app.help_text());
    } else if let Some(line) = app.prompt_line() {
        draw_prompt(frame, area, &line);
    } else if let Some(line) = app.confirm_line() {
        draw_prompt(frame, area, &line);
    }
}

fn draw_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = Pane::ALL.iter().map(|p| Line::from(p.title())).collect();
    let tabs = Tabs::new(titles)
        .select(app.pane.index())
        .block(Block::bordered().title("vissue"));
    frame.render_widget(tabs, area);
}

fn draw_list(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines = Vec::new();
    if app.rows.is_empty() {
        lines.push(Line::from("(empty)"));
    } else {
        for (i, row) in app.rows.iter().enumerate() {
            let marker = if i == app.selected { ">" } else { " " };
            let text = if row.extra.is_empty() {
                format!(
                    "{marker} {} [{:>8}] [#{}] {}  {}",
                    row.id, row.state, row.priority, row.title, row.project
                )
            } else {
                format!(
                    "{marker} {} [{:>8}] [#{}] {}  {}  {}",
                    row.id, row.state, row.priority, row.title, row.project, row.extra
                )
            };
            let style = if i == app.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(text, style)));
        }
    }
    let block = Block::bordered().title(app.pane.title());
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_detail(frame: &mut Frame, area: Rect, app: &App) {
    let title = format!("detail: {}", app.detail_tab.title());
    let id = app.selected_id().unwrap_or("-");
    let mut text = id.to_string();
    text.push('\n');
    text.push_str(&app.detail_body);
    let block = Block::bordered().title(title);
    frame.render_widget(
        Paragraph::new(text).block(block).wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(Paragraph::new(app.status_line()), area);
}

fn draw_prompt(frame: &mut Frame, area: Rect, line: &str) {
    let box_area = prompt_area(area, 3);
    frame.render_widget(Clear, box_area);
    frame.render_widget(
        Paragraph::new(line).block(Block::bordered().title("input")),
        box_area,
    );
}

fn draw_overlay(frame: &mut Frame, area: Rect, text: &str) {
    let box_area = centered(area, 70, 80);
    frame.render_widget(Clear, box_area);
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::bordered().title("help").borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        box_area,
    );
}

fn prompt_area(area: Rect, height: u16) -> Rect {
    let y = area.y + area.height.saturating_sub(height + 1);
    Rect {
        x: area.x + 2,
        y,
        width: area.width.saturating_sub(4),
        height,
    }
}

fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let width = area.width.saturating_mul(pct_x) / 100;
    let height = area.height.saturating_mul(pct_y) / 100;
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// Render `app` on a [`TestBackend`] and return the buffer as plain text.
///
/// # Errors
///
/// Returns an error if the test terminal cannot be created or drawn.
pub fn render_plain(
    app: &App,
    width: u16,
    height: u16,
) -> Result<String, vissue_core::error::Error> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).map_err(io_to_core)?;
    terminal.draw(|f| draw(f, app)).map_err(io_to_core)?;
    Ok(buffer_plain(terminal.backend()))
}

fn io_to_core<E: std::fmt::Display>(err: E) -> vissue_core::error::Error {
    vissue_core::error::Error::Other(anyhow::anyhow!("{err}"))
}

/// Flatten a test buffer to trimmed lines joined by `\n`.
pub fn buffer_plain(backend: &TestBackend) -> String {
    let buf = backend.buffer();
    let area = buf.area();
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        for x in area.left()..area.right() {
            line.push_str(buf[(x, y)].symbol());
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}
