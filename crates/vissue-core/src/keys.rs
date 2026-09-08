//! Action catalog and keys.toml overlay.
//!
//! Defaults live in code. Operator diffs live in `$VISSUE_KEYS` or
//! `~/.config/vissue/keys.toml`. Invalid overlay is refused.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Dotted action id. HUD-shaped, remappable except reserved chords.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionId {
    /// Move the list cursor down.
    ListDown,
    /// Move the list cursor up.
    ListUp,
    /// Open the selected issue.
    ListSelect,
    /// Toggle done on the selected issue.
    ListDone,
    /// Switch to the ready pane.
    PaneReady,
    /// Switch to the full list pane.
    PaneList,
    /// Switch to the claims pane.
    PaneClaims,
    /// Switch to the agenda pane.
    PaneAgenda,
    /// Switch to the search pane.
    PaneSearch,
    /// Cycle to the next pane.
    PaneNext,
    /// Cycle the detail card.
    DetailCycle,
    /// Cycle the project filter.
    ProjectCycle,
    /// Focus the board search box.
    Search,
    /// Create an issue.
    Add,
    /// Claim the selected issue.
    Claim,
    /// Append a note to the selected issue.
    Note,
    /// Cite a deed on the selected issue.
    Deed,
    /// Cycle the selected issue's state.
    StateCycle,
    /// Confirm marking the selected issue done.
    ConfirmDone,
    /// Confirm cancelling the selected issue.
    ConfirmCancel,
    /// Open the selected issue in an editor.
    Open,
    /// Copy the selected issue id.
    CopyId,
    /// Reload the board from disk.
    Reload,
    /// Show the key help overlay.
    Help,
}

impl ActionId {
    /// Stable dotted id used in `keys.toml` and help text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ListDown => "list.down",
            Self::ListUp => "list.up",
            Self::ListSelect => "list.select",
            Self::ListDone => "list.done",
            Self::PaneReady => "pane.ready",
            Self::PaneList => "pane.list",
            Self::PaneClaims => "pane.claims",
            Self::PaneAgenda => "pane.agenda",
            Self::PaneSearch => "pane.search",
            Self::PaneNext => "pane.next",
            Self::DetailCycle => "detail.cycle",
            Self::ProjectCycle => "project.cycle",
            Self::Search => "board.search",
            Self::Add => "issue.add",
            Self::Claim => "issue.claim",
            Self::Note => "issue.note",
            Self::Deed => "issue.deed",
            Self::StateCycle => "issue.state",
            Self::ConfirmDone => "issue.done",
            Self::ConfirmCancel => "issue.cancel",
            Self::Open => "issue.open",
            Self::CopyId => "issue.copy",
            Self::Reload => "board.reload",
            Self::Help => "board.help",
        }
    }

    /// Parse a dotted id such as `list.down`. Unknown names yield `None`.
    pub fn parse(raw: &str) -> Option<Self> {
        ALL.iter().find(|a| a.as_str() == raw).copied()
    }
}

/// Where an action is bound: always, or only on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Available in every HUD context.
    Global,
    /// Available while the issue board has focus.
    Board,
}

impl Scope {
    /// Stable scope name used in the key table.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Board => "board",
        }
    }
}

const ALL: &[ActionId] = &[
    ActionId::ListDown,
    ActionId::ListUp,
    ActionId::ListSelect,
    ActionId::ListDone,
    ActionId::PaneReady,
    ActionId::PaneList,
    ActionId::PaneClaims,
    ActionId::PaneAgenda,
    ActionId::PaneSearch,
    ActionId::PaneNext,
    ActionId::DetailCycle,
    ActionId::ProjectCycle,
    ActionId::Search,
    ActionId::Add,
    ActionId::Claim,
    ActionId::Note,
    ActionId::Deed,
    ActionId::StateCycle,
    ActionId::ConfirmDone,
    ActionId::ConfirmCancel,
    ActionId::Open,
    ActionId::CopyId,
    ActionId::Reload,
    ActionId::Help,
];

/// One catalog row. Defaults stay in this table.
#[derive(Debug, Clone, Copy)]
pub struct ActionRow {
    /// Action this row describes.
    pub id: ActionId,
    /// Context the default chord is bound in.
    pub scope: Scope,
    /// Compiled-in chord token.
    pub default: &'static str,
    /// Whether an overlay may rebind this action.
    pub remappable: bool,
}

const CATALOG: &[ActionRow] = &[
    ActionRow {
        id: ActionId::ListDown,
        scope: Scope::Board,
        default: "j",
        remappable: true,
    },
    ActionRow {
        id: ActionId::ListUp,
        scope: Scope::Board,
        default: "k",
        remappable: true,
    },
    ActionRow {
        id: ActionId::ListSelect,
        scope: Scope::Board,
        default: "enter",
        remappable: false,
    },
    ActionRow {
        id: ActionId::ListDone,
        scope: Scope::Board,
        default: "space",
        remappable: true,
    },
    ActionRow {
        id: ActionId::PaneReady,
        scope: Scope::Board,
        default: "1",
        remappable: true,
    },
    ActionRow {
        id: ActionId::PaneList,
        scope: Scope::Board,
        default: "2",
        remappable: true,
    },
    ActionRow {
        id: ActionId::PaneClaims,
        scope: Scope::Board,
        default: "3",
        remappable: true,
    },
    ActionRow {
        id: ActionId::PaneAgenda,
        scope: Scope::Board,
        default: "4",
        remappable: true,
    },
    ActionRow {
        id: ActionId::PaneSearch,
        scope: Scope::Board,
        default: "5",
        remappable: true,
    },
    ActionRow {
        id: ActionId::PaneNext,
        scope: Scope::Board,
        default: "tab",
        remappable: false,
    },
    ActionRow {
        id: ActionId::DetailCycle,
        scope: Scope::Board,
        default: "enter",
        remappable: false,
    },
    ActionRow {
        id: ActionId::ProjectCycle,
        scope: Scope::Board,
        default: "p",
        remappable: true,
    },
    ActionRow {
        id: ActionId::Search,
        scope: Scope::Board,
        default: "/",
        remappable: true,
    },
    ActionRow {
        id: ActionId::Add,
        scope: Scope::Board,
        default: "a",
        remappable: true,
    },
    ActionRow {
        id: ActionId::Claim,
        scope: Scope::Board,
        default: "c",
        remappable: true,
    },
    ActionRow {
        id: ActionId::Deed,
        scope: Scope::Board,
        default: "d",
        remappable: true,
    },
    ActionRow {
        id: ActionId::Note,
        scope: Scope::Board,
        default: "n",
        remappable: true,
    },
    ActionRow {
        id: ActionId::StateCycle,
        scope: Scope::Board,
        default: "s",
        remappable: true,
    },
    ActionRow {
        id: ActionId::ConfirmDone,
        scope: Scope::Board,
        default: "D",
        remappable: true,
    },
    ActionRow {
        id: ActionId::ConfirmCancel,
        scope: Scope::Board,
        default: "X",
        remappable: true,
    },
    ActionRow {
        id: ActionId::Open,
        scope: Scope::Board,
        default: "o",
        remappable: true,
    },
    ActionRow {
        id: ActionId::CopyId,
        scope: Scope::Board,
        default: "y",
        remappable: true,
    },
    ActionRow {
        id: ActionId::Reload,
        scope: Scope::Board,
        default: "R",
        remappable: true,
    },
    ActionRow {
        id: ActionId::Help,
        scope: Scope::Global,
        default: "?",
        remappable: false,
    },
];

/// Reserved chords the overlay may not steal.
const RESERVED: &[&str] = &["esc", "enter", "tab", "?"];

/// Resolved map: chord -> action (board scope).
#[derive(Debug, Clone)]
pub struct KeyMap {
    by_chord: BTreeMap<String, ActionId>,
    /// Optional leader character from the overlay.
    pub leader: Option<char>,
    /// How long a leader chord stays armed, in milliseconds.
    pub leader_timeout_ms: u64,
    /// Why the overlay was refused, when defaults were kept instead.
    pub overlay_error: Option<String>,
}

impl Default for KeyMap {
    fn default() -> Self {
        Self::from_defaults()
    }
}

impl KeyMap {
    /// Compiled-in chords, no overlay.
    pub fn from_defaults() -> Self {
        let mut by_chord = BTreeMap::new();
        for row in CATALOG {
            by_chord.insert(row.default.to_string(), row.id);
        }
        Self {
            by_chord,
            leader: None,
            leader_timeout_ms: 800,
            overlay_error: None,
        }
    }

    /// Load `$VISSUE_KEYS` or `~/.config/vissue/keys.toml` over the defaults.
    ///
    /// A missing file keeps the defaults. A broken overlay also keeps the
    /// defaults and records the reason in [`Self::overlay_error`].
    pub fn load() -> Self {
        let path = overlay_path();
        match path {
            Some(p) if p.is_file() => match load_overlay(&p) {
                Ok(map) => map,
                Err(err) => {
                    let mut map = Self::from_defaults();
                    map.overlay_error = Some(err);
                    map
                }
            },
            _ => Self::from_defaults(),
        }
    }

    /// Action bound to `chord` in board scope, if any.
    pub fn get(&self, chord: &str) -> Option<ActionId> {
        self.by_chord.get(chord).copied()
    }

    /// Help overlay rows: resolved chord, then the dotted action id.
    pub fn help_lines(&self) -> Vec<String> {
        CATALOG
            .iter()
            .map(|row| {
                let chord = self
                    .by_chord
                    .iter()
                    .find(|(_, id)| **id == row.id)
                    .map(|(c, _)| c.as_str())
                    .unwrap_or(row.default);
                format!("{chord:8}  {}", row.id.as_str())
            })
            .collect()
    }

    /// Every bound chord and the dotted action id it maps to.
    pub fn occupancy(&self) -> Vec<(String, String)> {
        self.by_chord
            .iter()
            .map(|(c, id)| (c.clone(), id.as_str().to_string()))
            .collect()
    }

    /// One line per action: scope, id, resolved chord.
    pub fn table_lines(&self) -> Vec<String> {
        CATALOG
            .iter()
            .map(|row| {
                let chord = self
                    .by_chord
                    .iter()
                    .find(|(_, id)| **id == row.id)
                    .map(|(c, _)| c.as_str())
                    .unwrap_or(row.default);
                format!("{:<8} {:<16} {chord}", row.scope.as_str(), row.id.as_str())
            })
            .collect()
    }
}

fn overlay_path() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("VISSUE_KEYS") {
        let t = raw.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("vissue/keys.toml"))
}

fn load_overlay(path: &Path) -> Result<KeyMap, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let value: toml::Value = text.parse().map_err(|e| format!("keys.toml: {e}"))?;
    let mut map = KeyMap::from_defaults();
    if let Some(leader) = value.get("leader").and_then(|v| v.as_str()) {
        let mut chars = leader.chars();
        let ch = chars.next();
        if ch.is_none() || chars.next().is_some() {
            return Err("leader must be one character".into());
        }
        map.leader = ch;
    }
    if let Some(ms) = value.get("leader_timeout_ms").and_then(|v| v.as_integer())
        && ms > 0
    {
        map.leader_timeout_ms = ms as u64;
    }
    let table = value.get("board").and_then(|v| v.as_table());
    if let Some(table) = table {
        let mut pending: Vec<(ActionId, String)> = Vec::new();
        for (id, chord) in table {
            let Some(action) = ActionId::parse(id) else {
                return Err(format!("unknown action {id}"));
            };
            let Some(row) = CATALOG.iter().find(|r| r.id == action) else {
                return Err(format!("unknown action {id}"));
            };
            let Some(chord) = chord.as_str() else {
                return Err(format!("{id} chord must be a string"));
            };
            if !row.remappable {
                return Err(format!("{id} is not remappable"));
            }
            if RESERVED.contains(&chord.to_ascii_lowercase().as_str()) {
                return Err(format!("cannot steal reserved chord {chord}"));
            }
            pending.push((action, chord.to_string()));
        }
        for (action, _) in &pending {
            map.by_chord.retain(|_, id| id != action);
        }
        for (action, chord) in pending {
            if let Some(prev) = map.by_chord.insert(chord.clone(), action) {
                return Err(format!("chord {chord} already bound to {}", prev.as_str()));
            }
        }
    }
    Ok(map)
}

/// Map a typed character (no modifiers) or a named key to a chord token.
pub fn chord_from_char(c: char) -> String {
    c.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay(body: &str) -> Result<KeyMap, String> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("keys.toml");
        std::fs::write(&path, body).expect("write");
        load_overlay(&path)
    }

    #[test]
    fn an_overlay_rebinds_only_what_it_names() {
        let map = overlay("[board]\n\"list.down\" = \"e\"\n").expect("overlay");
        assert_eq!(map.get("e"), Some(ActionId::ListDown));
        // The old chord is freed rather than left pointing at the action.
        assert_eq!(map.get("j"), None);
        // Everything it did not mention keeps its default.
        assert_eq!(map.get("k"), Some(ActionId::ListUp));
        assert_eq!(map.get("c"), Some(ActionId::Claim));
    }

    #[test]
    fn two_actions_may_swap_chords_in_one_overlay() {
        // Neither assignment is a conflict, because both old chords are
        // released before either new one is placed.
        let map =
            overlay("[board]\n\"list.down\" = \"k\"\n\"list.up\" = \"j\"\n").expect("overlay");
        assert_eq!(map.get("k"), Some(ActionId::ListDown));
        assert_eq!(map.get("j"), Some(ActionId::ListUp));
    }

    #[test]
    fn a_leader_is_one_character() {
        let map = overlay("leader = \",\"\n").expect("overlay");
        assert_eq!(map.leader, Some(','));
        assert!(overlay("leader = \"\"\n").is_err());
        assert!(overlay("leader = \"gg\"\n").is_err());
    }

    #[test]
    fn the_leader_timeout_takes_a_positive_number_only() {
        let map = overlay("leader_timeout_ms = 250\n").expect("overlay");
        assert_eq!(map.leader_timeout_ms, 250);
        // A nonsense value leaves the default standing rather than failing.
        let map = overlay("leader_timeout_ms = 0\n").expect("overlay");
        assert_eq!(
            map.leader_timeout_ms,
            KeyMap::from_defaults().leader_timeout_ms
        );
    }

    #[test]
    fn an_overlay_that_cannot_be_understood_says_which_part() {
        let unknown = overlay("[board]\n\"list.sideways\" = \"z\"\n").unwrap_err();
        assert!(unknown.contains("list.sideways"), "{unknown}");

        let not_a_string = overlay("[board]\n\"list.down\" = 3\n").unwrap_err();
        assert!(not_a_string.contains("list.down"), "{not_a_string}");

        let broken = overlay("[board\n").unwrap_err();
        assert!(broken.contains("keys.toml"), "{broken}");
    }

    #[test]
    fn the_keys_a_reader_needs_cannot_be_taken_away() {
        // Enter, tab, escape and ? are how someone gets out of a pane, so
        // neither the action nor the chord may be reassigned.
        let fixed = overlay("[board]\n\"list.select\" = \"z\"\n").unwrap_err();
        assert!(fixed.contains("not remappable"), "{fixed}");

        for reserved in ["enter", "tab", "esc", "?"] {
            let err = overlay(&format!("[board]\n\"list.down\" = \"{reserved}\"\n")).unwrap_err();
            assert!(err.contains("reserved"), "{reserved}: {err}");
        }
    }

    #[test]
    fn two_actions_may_not_share_one_chord() {
        let err = overlay("[board]\n\"list.down\" = \"c\"\n").unwrap_err();
        assert!(err.contains("already bound"), "{err}");
        assert!(err.contains("issue.claim"), "{err}");
    }

    #[test]
    fn a_broken_overlay_leaves_the_defaults_and_says_why() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("keys.toml");
        std::fs::write(&path, "[board]\n\"list.down\" = \"enter\"\n").expect("write");

        // `load` never fails: a seat with a bad overlay still gets a board.
        let map = match load_overlay(&path) {
            Ok(_) => panic!("a reserved chord was accepted"),
            Err(err) => {
                let mut map = KeyMap::from_defaults();
                map.overlay_error = Some(err);
                map
            }
        };
        assert_eq!(map.get("j"), Some(ActionId::ListDown));
        assert!(map.overlay_error.is_some());
    }

    #[test]
    fn a_missing_overlay_is_not_an_error_worth_reporting() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(load_overlay(&dir.path().join("absent.toml")).is_err());
        // ... and `load` falls back rather than surfacing it.
        assert_eq!(KeyMap::from_defaults().get("j"), Some(ActionId::ListDown));
    }

    #[test]
    fn the_help_and_the_table_describe_every_action() {
        let map = KeyMap::from_defaults();
        let help = map.help_lines();
        let table = map.table_lines();
        assert_eq!(help.len(), CATALOG.len());
        assert_eq!(table.len(), CATALOG.len());
        for row in CATALOG {
            assert!(
                help.iter().any(|l| l.contains(row.id.as_str())),
                "{} missing from help",
                row.id.as_str()
            );
            assert!(
                table.iter().any(|l| l.contains(row.id.as_str())),
                "{} missing from the table",
                row.id.as_str()
            );
        }
    }

    #[test]
    fn the_help_shows_the_rebound_chord_rather_than_the_default() {
        let map = overlay("[board]\n\"list.down\" = \"e\"\n").expect("overlay");
        let line = map
            .help_lines()
            .into_iter()
            .find(|l| l.contains("list.down"))
            .expect("a line for list.down");
        assert!(line.starts_with('e'), "{line}");
    }

    #[test]
    fn occupancy_reports_one_entry_per_bound_chord() {
        let map = KeyMap::from_defaults();
        let occupancy = map.occupancy();
        assert_eq!(occupancy.len(), map.by_chord.len());
        for (chord, id) in &occupancy {
            assert!(!chord.is_empty());
            assert!(!id.is_empty());
        }
    }

    #[test]
    fn a_typed_character_is_its_own_chord() {
        assert_eq!(chord_from_char('j'), "j");
        assert_eq!(chord_from_char('?'), "?");
    }

    #[test]
    fn every_action_has_a_unique_id() {
        let mut seen = BTreeMap::new();
        for row in CATALOG {
            assert!(
                seen.insert(row.id.as_str(), row.id).is_none(),
                "duplicate {}",
                row.id.as_str()
            );
            assert_eq!(ActionId::parse(row.id.as_str()), Some(row.id));
        }
        assert_eq!(seen.len(), ALL.len());
    }

    #[test]
    fn defaults_resolve_j_and_n() {
        let map = KeyMap::from_defaults();
        assert_eq!(map.get("j"), Some(ActionId::ListDown));
        assert_eq!(map.get("n"), Some(ActionId::Note));
        assert_eq!(map.get("?"), Some(ActionId::Help));
    }

    #[test]
    fn overlay_rejects_reserved_and_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.toml");
        std::fs::write(&path, "[board]\n\"issue.note\" = \"esc\"\n").unwrap();
        let err = load_overlay(&path).unwrap_err();
        assert!(err.contains("reserved"), "{err}");
        std::fs::write(&path, "[board]\n\"no.such\" = \"z\"\n").unwrap();
        let err = load_overlay(&path).unwrap_err();
        assert!(err.contains("unknown"), "{err}");
    }

    #[test]
    fn overlay_remaps_list_down() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.toml");
        std::fs::write(
            &path,
            "leader = \";\"\n[board]\n\"list.down\" = \"n\"\n\"issue.note\" = \"leader+n\"\n",
        )
        .unwrap();
        let map = load_overlay(&path).unwrap();
        assert_eq!(map.leader, Some(';'));
        assert_eq!(map.get("n"), Some(ActionId::ListDown));
        assert_eq!(map.get("j"), None);
    }
}
