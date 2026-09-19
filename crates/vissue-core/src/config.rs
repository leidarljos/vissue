//! Root and layout resolution plus the optional on-disk configuration.
//!
//! A tracker lives under `<root>/<prefix>`, one directory per project, each
//! holding an `issues.org`. `root` comes from the caller, `ISSUE_ROOT`,
//! `VISSUE_ROOT`, the working directory when that is itself a tracker, the
//! seat's own `vissue/config.toml`, and otherwise the working directory.
//! `prefix` comes from the caller, `VISSUE_PREFIX`, `<root>/vissue.toml`, or
//! the `Software` default.

use anyhow::Context;

use crate::error::Result;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Directory under the root that holds one subdirectory per project.
pub const DEFAULT_PREFIX: &str = "Software";

/// Where the tracker lives: a root directory and the project prefix inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: PathBuf,
    prefix: String,
    /// Whether the root was the working directory rather than something the
    /// caller named. A guessed root that turns out to hold no tracker is the
    /// one case where an empty answer is a wrong answer.
    guessed: bool,
}

impl Layout {
    /// Build a layout from an explicit root and prefix.
    ///
    /// An empty prefix falls back to [`DEFAULT_PREFIX`].
    pub fn new(root: impl Into<PathBuf>, prefix: impl Into<String>) -> Self {
        let prefix = prefix.into();
        Self {
            root: root.into(),
            prefix: if prefix.is_empty() {
                DEFAULT_PREFIX.to_string()
            } else {
                prefix
            },
            guessed: false,
        }
    }

    /// Resolve from explicit arguments, falling back to the environment, the
    /// directory the caller stands in, the seat's own file, and finally the
    /// compiled defaults. The order is the caller, then the environment, then
    /// the working directory when that is a tracker, then the seat file.
    ///
    /// # Errors
    ///
    /// Returns an error if the current directory cannot be resolved, or if
    /// `<root>/vissue.toml` exists but cannot be read or parsed.
    pub fn resolve(root: Option<&Path>, prefix: Option<&str>) -> Result<Self> {
        let here = std::env::current_dir().context("resolve current directory as root")?;
        let (root, guessed) = choose_root(
            root,
            std::env::var_os("ISSUE_ROOT")
                .or_else(|| std::env::var_os("VISSUE_ROOT"))
                .map(PathBuf::from),
            &here,
            here.join("vissue.toml").is_file(),
            SeatConfig::path().as_deref().and_then(SeatConfig::read),
        );
        let prefix = match prefix {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => match std::env::var("VISSUE_PREFIX") {
                Ok(v) if !v.is_empty() => v,
                _ => RootConfig::load(&root)?
                    .prefix
                    .unwrap_or_else(|| DEFAULT_PREFIX.to_string()),
            },
        };
        let mut layout = Self::new(root, prefix);
        layout.guessed = guessed;
        Ok(layout)
    }

    /// Refuse a guessed root that holds no tracker; a named root is trusted.
    ///
    /// # Errors
    ///
    /// [`crate::error::Error::NotATracker`] when the root was the working
    /// directory and carries neither `vissue.toml` nor the prefix directory.
    pub fn require_tracker(&self) -> Result<()> {
        if !self.guessed || self.root.join("vissue.toml").is_file() || self.projects_dir().is_dir()
        {
            return Ok(());
        }
        Err(crate::error::Error::NotATracker {
            root: self.root.clone(),
            prefix: self.prefix.clone(),
        })
    }

    /// Tracker root: the directory that holds `vissue.toml` and `prefix`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory name under [`Self::root`] that holds one subdirectory per project.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// `<root>/<prefix>`: the directory scanned for projects.
    pub fn projects_dir(&self) -> PathBuf {
        self.root.join(&self.prefix)
    }

    /// `<root>/<prefix>/<project>/issues.org`.
    pub fn project_issues_path(&self, project: &str) -> PathBuf {
        self.projects_dir().join(project).join("issues.org")
    }
}

/// Which root the tracker is, and whether it was a guess (the working
/// directory for want of anything better), which [`Layout::require_tracker`]
/// refuses when empty.
fn choose_root(
    named: Option<&Path>,
    from_env: Option<PathBuf>,
    here: &Path,
    here_is_a_tracker: bool,
    seat: Option<PathBuf>,
) -> (PathBuf, bool) {
    if let Some(root) = named {
        return (root.to_path_buf(), false);
    }
    if let Some(root) = from_env {
        return (root, false);
    }
    // Standing in a tracker means that tracker, whatever the seat file says:
    // the caller is the more specific of the two.
    if here_is_a_tracker {
        return (here.to_path_buf(), false);
    }
    match seat {
        Some(root) => (root, false),
        None => (here.to_path_buf(), true),
    }
}

/// The seat's own configuration file, the one the router reads: which tracker
/// it means when nobody says. Only `root` is read here.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct SeatConfig {
    root: Option<String>,
}

impl SeatConfig {
    /// The configured root, or nothing; an unreadable file is nothing too.
    fn read(path: &Path) -> Option<PathBuf> {
        let raw = fs::read_to_string(path).ok()?;
        let parsed: Self = toml::from_str(&raw).ok()?;
        let named = parsed.root?;
        let named = named.trim();
        if named.is_empty() {
            return None;
        }
        let expanded = match named.strip_prefix("~/") {
            Some(rest) => home()?.join(rest),
            None => PathBuf::from(named),
        };
        expanded.is_dir().then_some(expanded)
    }

    /// `$VISSUE_CONFIG`, else the file under the seat's configuration
    /// directory. The same two the router looks at, in the same order.
    fn path() -> Option<PathBuf> {
        if let Some(named) = std::env::var_os("VISSUE_CONFIG").filter(|raw| !raw.is_empty()) {
            return Some(PathBuf::from(named));
        }
        let base = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => home()?.join(".config"),
        };
        Some(base.join("vissue").join("config.toml"))
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The tracker at `root` as its own `vissue.toml` describes it: the prefix
/// it names, else the default. Named, not guessed.
///
/// # Errors
///
/// A `vissue.toml` that cannot be read or parsed.
pub fn layout_at(root: &Path) -> Result<Layout> {
    let prefix = RootConfig::load(root)?
        .prefix
        .unwrap_or_else(|| DEFAULT_PREFIX.to_string());
    Ok(Layout::new(root.to_path_buf(), prefix))
}

/// `<root>/vissue.toml`, the product-level configuration file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct RootConfig {
    prefix: Option<String>,
    agent: Option<String>,
    issues: IssuesOverride,
    consensus: ConsensusOverride,
}

impl RootConfig {
    fn load(root: &Path) -> Result<Self> {
        let path = root.join("vissue.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        toml::from_str(&raw)
            .with_context(|| format!("parse {}", path.display()))
            .map_err(crate::error::Error::from)
    }
}

/// Knobs that shape newly created issues.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct IssuesSection {
    /// Priority cookie applied when `create` is called without one.
    pub default_priority: char,
    /// Length in base36 characters of the random suffix in a generated id.
    pub id_length: usize,
    /// How long a claim may sit on a STARTED issue before hygiene calls it
    /// stale.
    pub stale_claim_days: i64,
    /// Whether `hygiene` reports work that closed citing no deed. Off by default.
    pub expect_deeds: bool,
}

impl Default for IssuesSection {
    fn default() -> Self {
        Self {
            default_priority: 'C',
            id_length: 4,
            stale_claim_days: 7,
            expect_deeds: false,
        }
    }
}

/// The subset of [`IssuesSection`] a configuration file names. A key left out
/// of a file stays whatever the layer below it set, so a file that tunes one
/// knob does not silently reset the others.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct IssuesOverride {
    default_priority: Option<char>,
    id_length: Option<usize>,
    stale_claim_days: Option<i64>,
    expect_deeds: Option<bool>,
}

impl IssuesOverride {
    fn apply_to(&self, base: &mut IssuesSection) {
        if let Some(value) = self.default_priority {
            base.default_priority = value;
        }
        if let Some(value) = self.id_length {
            base.id_length = value;
        }
        if let Some(value) = self.stale_claim_days {
            base.stale_claim_days = value;
        }
        if let Some(value) = self.expect_deeds {
            base.expect_deeds = value;
        }
    }
}

/// Who listens to whom, and how hard the consensus iteration tries. Rows are
/// normalised by [`crate::consensus`]; an agent with no row keeps
/// `self_weight` and splits the rest equally.
///
/// ```toml
/// [consensus]
/// self_weight = 0.5
/// susceptibility = 0.8
///
/// [consensus.trust]
/// reviewer = { maintainer = 3.0, worker = 1.0 }
/// worker = { maintainer = 1.0 }
///
/// [consensus.susceptibility_of]
/// maintainer = 0.2
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ConsensusSection {
    /// Weight an agent puts on its own opinion when its row does not name it.
    pub self_weight: f64,
    /// How far an agent moves off the ballot it cast, in `[0, 1]`: one is
    /// DeGroot (the default), below one is Friedkin-Johnsen.
    pub susceptibility: f64,
    /// Largest disagreement that still counts as settled.
    pub tolerance: f64,
    /// Rounds to try before calling the trust graph periodic.
    pub max_iterations: usize,
    /// Susceptibility for one named agent; others use [`Self::susceptibility`].
    pub susceptibility_of: BTreeMap<String, f64>,
    /// Trust rows, keyed by the identity that holds the opinion.
    pub trust: BTreeMap<String, BTreeMap<String, f64>>,
}

impl Default for ConsensusSection {
    fn default() -> Self {
        Self {
            // Positive on purpose. A zero diagonal is what makes a trust graph
            // periodic, and a tracker nobody has configured should converge.
            self_weight: 0.5,
            susceptibility: 1.0,
            susceptibility_of: BTreeMap::new(),
            tolerance: 1e-9,
            max_iterations: 500,
            trust: BTreeMap::new(),
        }
    }
}

/// The subset of [`ConsensusSection`] a configuration file names.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct ConsensusOverride {
    self_weight: Option<f64>,
    susceptibility: Option<f64>,
    #[serde(default)]
    susceptibility_of: BTreeMap<String, f64>,
    tolerance: Option<f64>,
    max_iterations: Option<usize>,
    trust: BTreeMap<String, BTreeMap<String, f64>>,
}

impl ConsensusOverride {
    /// Apply this layer, refusing rather than clamping values the iteration
    /// cannot use.
    fn apply_to(&self, base: &mut ConsensusSection, whence: &Path) -> Result<()> {
        if let Some(value) = self.self_weight {
            if !(0.0..=1.0).contains(&value) {
                return Err(anyhow::anyhow!(
                    "{}: consensus.self_weight is {value}, which is not a share between 0 and 1",
                    whence.display()
                )
                .into());
            }
            base.self_weight = value;
        }
        if let Some(value) = self.susceptibility {
            if !(0.0..=1.0).contains(&value) {
                return Err(anyhow::anyhow!(
                    "{}: consensus.susceptibility is {value}, which is not a share between 0 and 1",
                    whence.display()
                )
                .into());
            }
            base.susceptibility = value;
        }
        for (agent, value) in &self.susceptibility_of {
            if !(0.0..=1.0).contains(value) {
                return Err(anyhow::anyhow!(
                    "{}: consensus.susceptibility_of.{agent} is {value}, \
                     which is not a share between 0 and 1",
                    whence.display()
                )
                .into());
            }
            // Agent by agent, like the trust rows: a file that pins one
            // reviewer does not drop the others.
            base.susceptibility_of.insert(agent.clone(), *value);
        }
        if let Some(value) = self.tolerance {
            if !(value > 0.0 && value.is_finite()) {
                return Err(anyhow::anyhow!(
                    "{}: consensus.tolerance is {value}, which is not a positive distance",
                    whence.display()
                )
                .into());
            }
            base.tolerance = value;
        }
        if let Some(value) = self.max_iterations {
            if value == 0 {
                return Err(anyhow::anyhow!(
                    "{}: consensus.max_iterations is 0, which runs no rounds at all",
                    whence.display()
                )
                .into());
            }
            base.max_iterations = value;
        }
        for (agent, row) in &self.trust {
            for (other, weight) in row {
                if !(*weight >= 0.0 && weight.is_finite()) {
                    return Err(anyhow::anyhow!(
                        "{}: consensus.trust.{agent}.{other} is {weight}, \
                         which is not a weight",
                        whence.display()
                    )
                    .into());
                }
            }
            // Row by row, like every other override: a file that retunes one
            // agent's trust does not silently drop the rows it says nothing
            // about.
            base.trust.insert(agent.clone(), row.clone());
        }
        Ok(())
    }
}

/// Effective configuration for one layout.
#[derive(Debug, Clone, Default)]
pub struct VissueConfig {
    /// Knobs that shape newly created issues and hygiene thresholds.
    pub issues: IssuesSection,
    /// Who listens to whom when a consensus is computed.
    pub consensus: ConsensusSection,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct PrefixConfigFile {
    issues: IssuesOverride,
    consensus: ConsensusOverride,
}

impl VissueConfig {
    /// `<root>/<prefix>/issues.config.toml` overrides `<root>/vissue.toml`,
    /// which overrides the compiled defaults. Neither file is required, and
    /// each layer overrides key by key rather than wholesale.
    ///
    /// # Errors
    ///
    /// Returns an error if a configuration file exists but cannot be read or
    /// parsed.
    pub fn load(layout: &Layout) -> Result<Self> {
        let mut issues = IssuesSection::default();
        let mut consensus = ConsensusSection::default();
        let root_path = layout.root().join("vissue.toml");
        let root = RootConfig::load(layout.root())?;
        root.issues.apply_to(&mut issues);
        root.consensus.apply_to(&mut consensus, &root_path)?;
        let path = layout.projects_dir().join("issues.config.toml");
        if path.exists() {
            let raw =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            let parsed: PrefixConfigFile =
                toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
            parsed.issues.apply_to(&mut issues);
            parsed.consensus.apply_to(&mut consensus, &path)?;
        }
        Ok(Self { issues, consensus })
    }
}

/// Who is claiming work here.
///
/// `VISSUE_AGENT` wins, then `agent` in `<root>/vissue.toml`, then
/// `user@host`. The value is opaque: an agent should set `VISSUE_AGENT` to
/// something stable enough to identify it across sessions, such as a model
/// and session tag, and any string it picks is stored verbatim.
pub fn identity(layout: &Layout) -> String {
    if let Ok(value) = crate::process_env::var("VISSUE_AGENT") {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    if let Ok(cfg) = RootConfig::load(layout.root())
        && let Some(agent) = cfg.agent
    {
        let agent = agent.trim().to_string();
        if !agent.is_empty() {
            return agent;
        }
    }
    format!("{}@{}", current_user(), current_host())
}

fn current_user() -> String {
    for var in ["USER", "LOGNAME", "USERNAME"] {
        if let Ok(value) = std::env::var(var)
            && !value.trim().is_empty()
        {
            return value.trim().to_string();
        }
    }
    "unknown".to_string()
}

fn current_host() -> String {
    if let Ok(value) = std::env::var("HOSTNAME")
        && !value.trim().is_empty()
    {
        return value.trim().to_string();
    }
    // HOSTNAME is not exported by every shell, so fall back to the file the
    // system keeps it in.
    for path in ["/etc/hostname", "/proc/sys/kernel/hostname"] {
        if let Ok(text) = fs::read_to_string(path) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "unknown".to_string()
}

#[cfg(test)]
#[allow(deprecated_safe_2024)]
mod tests {
    use super::*;

    #[test]
    fn layout_defaults_to_software_prefix() {
        let layout = Layout::new("/somewhere", "");
        assert_eq!(layout.prefix(), DEFAULT_PREFIX);
        assert_eq!(
            layout.project_issues_path("demo"),
            Path::new("/somewhere/Software/demo/issues.org")
        );
    }

    #[test]
    fn explicit_prefix_wins() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("vissue.toml"), "prefix = \"projects\"\n").unwrap();
        let layout = Layout::resolve(Some(dir.path()), Some("tracker")).unwrap();
        assert_eq!(layout.prefix(), "tracker");
    }

    #[test]
    fn root_config_supplies_prefix() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("vissue.toml"), "prefix = \"projects\"\n").unwrap();
        let layout = Layout::resolve(Some(dir.path()), None).unwrap();
        assert_eq!(layout.prefix(), "projects");
        assert_eq!(
            layout.projects_dir(),
            dir.path().join("projects"),
            "projects dir follows the configured prefix"
        );
    }

    /// `VISSUE_AGENT` is process-global, so the identity tests take turns.
    static AGENT_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn the_environment_names_the_claiming_identity_first() {
        let _guard = AGENT_ENV.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("vissue.toml"), "agent = \"from-file\"\n").unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);

        crate::process_env::override_var("VISSUE_AGENT", Some("from-env"));
        let from_env = identity(&layout);
        crate::process_env::override_var("VISSUE_AGENT", Some("   "));
        let blank_falls_through = identity(&layout);
        crate::process_env::override_var("VISSUE_AGENT", None);
        let from_file = identity(&layout);
        crate::process_env::clear_override("VISSUE_AGENT");

        assert_eq!(from_env, "from-env");
        assert_eq!(
            blank_falls_through, "from-file",
            "a blank value is not an identity"
        );
        assert_eq!(from_file, "from-file");
    }

    #[test]
    fn without_configuration_the_identity_is_user_at_host() {
        let _guard = AGENT_ENV.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        crate::process_env::override_var("VISSUE_AGENT", None);
        let resolved = identity(&layout);
        crate::process_env::clear_override("VISSUE_AGENT");
        assert!(resolved.contains('@'), "{resolved}");
        assert!(!resolved.starts_with('@'), "{resolved}");
        assert!(!resolved.ends_with('@'), "{resolved}");
    }

    #[test]
    fn the_stale_claim_threshold_is_configurable() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        assert_eq!(
            VissueConfig::load(&layout).unwrap().issues.stale_claim_days,
            7
        );

        fs::write(
            dir.path().join("vissue.toml"),
            "[issues]\nstale_claim_days = 3\n",
        )
        .unwrap();
        assert_eq!(
            VissueConfig::load(&layout).unwrap().issues.stale_claim_days,
            3
        );
    }

    /// A weight the iteration cannot use is refused rather than clamped. A
    /// `self_weight` of 2 is a typo, and clamping it to 1 would hand back a
    /// consensus in which nobody listened to anybody and say nothing about why.
    #[test]
    fn a_consensus_weight_the_iteration_cannot_use_is_refused() {
        for (body, wanted) in [
            ("[consensus]\nself_weight = 2.0\n", "self_weight"),
            ("[consensus]\nself_weight = -0.5\n", "self_weight"),
            ("[consensus]\ntolerance = 0.0\n", "tolerance"),
            ("[consensus]\ntolerance = -1.0\n", "tolerance"),
            ("[consensus]\nmax_iterations = 0\n", "max_iterations"),
            (
                "[consensus.trust]\nalice = { bob = -1.0 }\n",
                "consensus.trust.alice.bob",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("vissue.toml"), body).unwrap();
            let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
            let err = VissueConfig::load(&layout).unwrap_err().to_string();
            assert!(err.contains(wanted), "{body:?} -> {err}");
            assert!(
                err.contains("vissue.toml"),
                "the message has to name the file: {err}"
            );
        }
    }

    /// A per-agent susceptibility outside the range is refused the same way the
    /// default is, and the message names the agent as well as the file, because
    /// a table of reviewers needs to say which row is wrong.
    #[test]
    fn a_per_agent_susceptibility_is_checked_and_names_the_agent() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("vissue.toml"),
            "[consensus.susceptibility_of]\nmaintainer = 1.5\n",
        )
        .unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        let err = VissueConfig::load(&layout).unwrap_err().to_string();
        assert!(err.contains("maintainer"), "{err}");
        assert!(err.contains("vissue.toml"), "{err}");
    }

    /// Susceptibility merges agent by agent, like the trust rows: a file that
    /// pins one reviewer must not drop the others.
    #[test]
    fn a_susceptibility_row_overrides_only_the_agent_it_names() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("vissue.toml"),
            "[consensus.susceptibility_of]\nmaintainer = 0.2\nreviewer = 0.6\n",
        )
        .unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        fs::create_dir_all(layout.projects_dir()).unwrap();
        fs::write(
            layout.projects_dir().join("issues.config.toml"),
            "[consensus.susceptibility_of]\nmaintainer = 0.4\n",
        )
        .unwrap();

        let cfg = VissueConfig::load(&layout).unwrap().consensus;
        assert_eq!(cfg.susceptibility_of.get("maintainer"), Some(&0.4));
        assert_eq!(
            cfg.susceptibility_of.get("reviewer"),
            Some(&0.6),
            "a row the second file says nothing about survives"
        );
    }

    /// The whole range is usable, ends included: zero self-weight is the
    /// periodic case the consensus report exists to name, and one is an agent
    /// that listens to nobody.
    #[test]
    fn the_ends_of_the_self_weight_range_are_accepted() {
        for value in ["0.0", "1.0"] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(
                dir.path().join("vissue.toml"),
                format!("[consensus]\nself_weight = {value}\n"),
            )
            .unwrap();
            let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
            let cfg = VissueConfig::load(&layout).expect(value);
            assert_eq!(cfg.consensus.self_weight, value.parse::<f64>().unwrap());
        }
    }

    /// Trust merges row by row, like every other override. A file that retunes
    /// one agent must not silently drop the rows it says nothing about.
    #[test]
    fn a_trust_row_overrides_only_the_agent_it_names() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("vissue.toml"),
            "[consensus.trust]\nalice = { bob = 1.0 }\ncarol = { alice = 1.0 }\n",
        )
        .unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        fs::create_dir_all(layout.projects_dir()).unwrap();
        fs::write(
            layout.projects_dir().join("issues.config.toml"),
            "[consensus.trust]\nalice = { carol = 4.0 }\n",
        )
        .unwrap();

        let cfg = VissueConfig::load(&layout).unwrap();
        assert_eq!(
            cfg.consensus
                .trust
                .get("alice")
                .and_then(|r| r.get("carol")),
            Some(&4.0),
            "the named row is replaced whole"
        );
        assert!(
            cfg.consensus
                .trust
                .get("alice")
                .is_some_and(|r| !r.contains_key("bob")),
            "replaced, not merged into: {:?}",
            cfg.consensus.trust
        );
        assert_eq!(
            cfg.consensus
                .trust
                .get("carol")
                .and_then(|r| r.get("alice")),
            Some(&1.0),
            "a row the second file says nothing about survives"
        );
    }

    /// Nothing configured is the shape the consensus verb reduces to a tally in.
    #[test]
    fn the_consensus_defaults_converge_on_their_own() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        let cfg = VissueConfig::load(&layout).unwrap().consensus;
        assert!(cfg.trust.is_empty());
        assert!(
            cfg.self_weight > 0.0,
            "a zero diagonal is what makes a trust graph periodic"
        );
        assert!(cfg.tolerance > 0.0 && cfg.max_iterations > 0);
    }

    #[test]
    fn config_defaults_when_no_files_present() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        let cfg = VissueConfig::load(&layout).unwrap();
        assert_eq!(cfg.issues.default_priority, 'C');
        assert_eq!(cfg.issues.id_length, 4);
    }

    #[test]
    fn prefix_scoped_config_overrides_root_config() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("vissue.toml"),
            "[issues]\ndefault_priority = \"B\"\nid_length = 5\n",
        )
        .unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        let cfg = VissueConfig::load(&layout).unwrap();
        assert_eq!(cfg.issues.default_priority, 'B');
        assert_eq!(cfg.issues.id_length, 5);

        fs::create_dir_all(layout.projects_dir()).unwrap();
        fs::write(
            layout.projects_dir().join("issues.config.toml"),
            "[issues]\ndefault_priority = \"A\"\nid_length = 6\n",
        )
        .unwrap();
        let cfg = VissueConfig::load(&layout).unwrap();
        assert_eq!(cfg.issues.default_priority, 'A');
        assert_eq!(cfg.issues.id_length, 6);
    }

    #[test]
    fn a_partial_override_keeps_the_keys_it_does_not_name() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("vissue.toml"),
            "[issues]\ndefault_priority = \"B\"\nid_length = 5\nstale_claim_days = 3\n",
        )
        .unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        fs::create_dir_all(layout.projects_dir()).unwrap();
        fs::write(
            layout.projects_dir().join("issues.config.toml"),
            "[issues]\nid_length = 6\n",
        )
        .unwrap();

        let cfg = VissueConfig::load(&layout).unwrap();
        assert_eq!(cfg.issues.id_length, 6, "the named key is overridden");
        assert_eq!(
            cfg.issues.default_priority, 'B',
            "an unnamed key keeps the root value"
        );
        assert_eq!(cfg.issues.stale_claim_days, 3);
    }

    /// A seat file names the tracker the bare command means.
    #[test]
    fn a_seat_file_names_a_tracker() {
        let dir = tempfile::tempdir().unwrap();
        let tracker = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            format!("root = {:?}\n", tracker.path().display().to_string()),
        )
        .unwrap();
        assert_eq!(
            SeatConfig::read(&path).unwrap().canonicalize().unwrap(),
            tracker.path().canonicalize().unwrap()
        );
    }

    /// A file that is absent, unparseable, empty, or names a directory that is
    /// not there says nothing rather than failing: this is the fallback path,
    /// and the working directory below it still gives the caller a message.
    #[test]
    fn a_seat_file_that_says_nothing_usable_says_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(SeatConfig::read(&dir.path().join("absent.toml")).is_none());
        for text in [
            "",
            "root = \"\"\n",
            "root = \"/nonexistent/tracker\"\n",
            "root =",
        ] {
            let path = dir.path().join("config.toml");
            fs::write(&path, text).unwrap();
            assert!(SeatConfig::read(&path).is_none(), "{text:?}");
        }
    }

    /// The router owns this file too, and the two read it for different keys.
    /// A seat that has routes still gets a root out of it, and the router
    /// still parses a file that names one.
    #[test]
    fn the_seat_root_shares_the_file_the_router_reads() {
        let dir = tempfile::tempdir().unwrap();
        let tracker = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            format!(
                "root = {:?}\n\n[layouts.other]\nroot = \"/somewhere\"\nprefix = \"Issues\"\n\n[routes]\nthing = \"other\"\n",
                tracker.path().display().to_string()
            ),
        )
        .unwrap();
        assert_eq!(
            SeatConfig::read(&path).unwrap().canonicalize().unwrap(),
            tracker.path().canonicalize().unwrap()
        );
        // The router's own parse is strict about unknown keys, so the same
        // bytes have to be readable by it: one file, two readers.
        crate::router::Router::from_file(Layout::new(dir.path(), DEFAULT_PREFIX), &path)
            .expect("the router reads the same file");
    }

    /// The order the root is decided in, with nothing global touched.
    #[test]
    fn the_caller_beats_the_environment_beats_where_you_stand() {
        let named = PathBuf::from("/named");
        let from_env = PathBuf::from("/env");
        let seat = PathBuf::from("/seat");
        let here = PathBuf::from("/here");

        // Something the caller named wins, and is never a guess.
        assert_eq!(
            choose_root(
                Some(&named),
                Some(from_env.clone()),
                &here,
                false,
                Some(seat.clone())
            ),
            (named.clone(), false)
        );
        // Then the environment.
        assert_eq!(
            choose_root(
                None,
                Some(from_env.clone()),
                &here,
                true,
                Some(seat.clone())
            ),
            (from_env, false)
        );
        // Standing in a tracker means that tracker, over the seat's default:
        // the caller is the more specific of the two.
        assert_eq!(
            choose_root(None, None, &here, true, Some(seat.clone())),
            (here.clone(), false)
        );
        // Standing nowhere in particular, the seat's own tracker.
        assert_eq!(
            choose_root(None, None, &here, false, Some(seat.clone())),
            (seat, false)
        );
        // And with no seat file, the working directory as a guess, which is
        // what `require_tracker` refuses when it holds no tracker.
        assert_eq!(choose_root(None, None, &here, false, None), (here, true));
    }
}
