//! Root and layout resolution plus the optional on-disk configuration.
//!
//! A tracker lives under `<root>/<prefix>`, one directory per project, each
//! holding an `issues.org`. `root` comes from the caller, `ISSUE_ROOT`,
//! `VISSUE_ROOT`, or the current directory. `prefix` comes from the caller,
//! `VISSUE_PREFIX`, `<root>/vissue.toml`, or the `Software` default.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

/// Directory under the root that holds one subdirectory per project.
pub const DEFAULT_PREFIX: &str = "Software";

/// Where the tracker lives: a root directory and the project prefix inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: PathBuf,
    prefix: String,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>, prefix: impl Into<String>) -> Self {
        let prefix = prefix.into();
        Self {
            root: root.into(),
            prefix: if prefix.is_empty() {
                DEFAULT_PREFIX.to_string()
            } else {
                prefix
            },
        }
    }

    /// Resolve from explicit arguments, falling back to the environment, the
    /// on-disk `vissue.toml`, and finally the compiled defaults.
    pub fn resolve(root: Option<&Path>, prefix: Option<&str>) -> Result<Self> {
        let root = match root {
            Some(p) => p.to_path_buf(),
            None => {
                match std::env::var_os("ISSUE_ROOT").or_else(|| std::env::var_os("VISSUE_ROOT")) {
                    Some(v) => PathBuf::from(v),
                    None => std::env::current_dir().context("resolve current directory as root")?,
                }
            }
        };
        let prefix = match prefix {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => match std::env::var("VISSUE_PREFIX") {
                Ok(v) if !v.is_empty() => v,
                _ => RootConfig::load(&root)?
                    .prefix
                    .unwrap_or_else(|| DEFAULT_PREFIX.to_string()),
            },
        };
        Ok(Self::new(root, prefix))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

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

/// `<root>/vissue.toml`, the product-level configuration file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct RootConfig {
    prefix: Option<String>,
    agent: Option<String>,
    issues: Option<IssuesSection>,
}

impl RootConfig {
    fn load(root: &Path) -> Result<Self> {
        let path = root.join("vissue.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))
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
}

impl Default for IssuesSection {
    fn default() -> Self {
        Self {
            default_priority: 'C',
            id_length: 4,
            stale_claim_days: 7,
        }
    }
}

/// Effective configuration for one layout.
#[derive(Debug, Clone, Default)]
pub struct VissueConfig {
    pub issues: IssuesSection,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct PrefixConfigFile {
    issues: IssuesSection,
}

impl VissueConfig {
    /// `<root>/<prefix>/issues.config.toml` overrides `<root>/vissue.toml`,
    /// which overrides the compiled defaults. Neither file is required.
    pub fn load(layout: &Layout) -> Result<Self> {
        let mut issues = RootConfig::load(layout.root())?.issues.unwrap_or_default();
        let path = layout.projects_dir().join("issues.config.toml");
        if path.exists() {
            let raw =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            let parsed: PrefixConfigFile =
                toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
            issues = parsed.issues;
        }
        Ok(Self { issues })
    }
}

/// Who is claiming work here.
///
/// `VISSUE_AGENT` wins, then `agent` in `<root>/vissue.toml`, then
/// `user@host`. The value is opaque: an agent should set `VISSUE_AGENT` to
/// something stable enough to identify it across sessions, such as a model
/// and session tag, and any string it picks is stored verbatim.
pub fn identity(layout: &Layout) -> String {
    if let Ok(value) = std::env::var("VISSUE_AGENT") {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    if let Ok(cfg) = RootConfig::load(layout.root()) {
        if let Some(agent) = cfg.agent {
            let agent = agent.trim().to_string();
            if !agent.is_empty() {
                return agent;
            }
        }
    }
    format!("{}@{}", current_user(), current_host())
}

fn current_user() -> String {
    for var in ["USER", "LOGNAME", "USERNAME"] {
        if let Ok(value) = std::env::var(var) {
            if !value.trim().is_empty() {
                return value.trim().to_string();
            }
        }
    }
    "unknown".to_string()
}

fn current_host() -> String {
    if let Ok(value) = std::env::var("HOSTNAME") {
        if !value.trim().is_empty() {
            return value.trim().to_string();
        }
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

        std::env::set_var("VISSUE_AGENT", "from-env");
        let from_env = identity(&layout);
        std::env::set_var("VISSUE_AGENT", "   ");
        let blank_falls_through = identity(&layout);
        std::env::remove_var("VISSUE_AGENT");
        let from_file = identity(&layout);

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
        std::env::remove_var("VISSUE_AGENT");
        let resolved = identity(&layout);
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
}
