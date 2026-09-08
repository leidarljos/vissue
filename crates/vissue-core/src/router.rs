//! User-level project router: named projects can live on another checkout.
//!
//! `Layout::resolve` is still the process default. A file at
//! `$VISSUE_CONFIG` or `$XDG_CONFIG_HOME/vissue/config.toml` maps a project
//! name onto a `{root, prefix}` plus an on-disk directory. A named route
//! wins over `--root` / `VISSUE_ROOT`, because callers that inject a vault
//! root still need those names to land in the routed file. `VISSUE_NO_ROUTE`
//! or a missing config file restores single-layout behaviour.

use anyhow::{Context, anyhow};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{DEFAULT_PREFIX, Layout};
use crate::error::{Error, Result};
use crate::model::IssueHeading;
use crate::process_env;
use crate::store::{self, IssueDoc};

/// One project as the router names it: the layout to read, the directory
/// under that layout's prefix, and the route key the caller used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRef {
    /// Tracker that holds the file.
    pub layout: Layout,
    /// Directory name under `layout.prefix()`, which is also the id prefix.
    pub dir: String,
    /// Route key or, when unrouted, the same string as `dir`.
    pub key: String,
}

/// A heading found by id, together with the layout that holds it.
#[derive(Debug, Clone)]
pub struct RouteHit {
    /// Layout that contains the heading.
    pub layout: Layout,
    /// Project directory the heading lives in.
    pub project: String,
    /// Parsed heading.
    pub heading: IssueHeading,
    /// Path of the `issues.org` that defined it.
    pub path: PathBuf,
}

/// User-level map of project name to layout, plus the process default.
#[derive(Debug, Clone)]
pub struct Router {
    default: Layout,
    /// Named layouts from `[layouts.*]`, excluding any that equal `default`.
    named: BTreeMap<String, Layout>,
    /// Lowercased route key -> (layout name or "default", on-disk directory).
    routes: BTreeMap<String, (String, String)>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct UserConfig {
    layouts: BTreeMap<String, LayoutSpec>,
    routes: BTreeMap<String, RouteSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutSpec {
    root: String,
    #[serde(default = "default_prefix_string")]
    prefix: String,
}

fn default_prefix_string() -> String {
    DEFAULT_PREFIX.to_string()
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RouteSpec {
    LayoutName(String),
    Table {
        layout: String,
        #[serde(default)]
        project_dir: Option<String>,
    },
}

impl Router {
    /// A router that never leaves `default`. Used when routing is off.
    #[must_use]
    pub fn unrouted(default: Layout) -> Self {
        Self {
            default,
            named: BTreeMap::new(),
            routes: BTreeMap::new(),
        }
    }

    /// Load `$VISSUE_CONFIG` or the XDG file over `default`.
    ///
    /// A missing default-path file is a no-op. `VISSUE_NO_ROUTE` set to a
    /// non-empty value other than `0` / `false` ignores the file.
    ///
    /// # Errors
    ///
    /// Returns an error if an explicit `VISSUE_CONFIG` path is missing, the
    /// file cannot be read or parsed, a route names an unknown layout, or a
    /// layout root is relative after expansion.
    pub fn load(default: Layout) -> Result<Self> {
        if routing_disabled() {
            return Ok(Self::unrouted(default));
        }
        let Some(path) = user_config_path() else {
            return Ok(Self::unrouted(default));
        };
        if !path.exists() {
            if process_env::var("VISSUE_CONFIG").is_ok() {
                return Err(anyhow!("VISSUE_CONFIG {} does not exist", path.display()).into());
            }
            return Ok(Self::unrouted(default));
        }
        Self::from_file(default, &path)
    }

    /// Load an explicit config file. Tests and callers that already resolved
    /// the path use this.
    ///
    /// # Errors
    ///
    /// Same as [`Self::load`].
    pub fn from_file(default: Layout, path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let parsed: UserConfig =
            toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        Self::from_config(default, parsed)
    }

    fn from_config(default: Layout, parsed: UserConfig) -> Result<Self> {
        let default_key = layout_key(&default);
        let mut named = BTreeMap::new();
        for (name, spec) in &parsed.layouts {
            let layout = Layout::new(expand_path(&spec.root)?, spec.prefix.clone());
            if layout_key(&layout) == default_key {
                continue;
            }
            named.insert(name.clone(), layout);
        }
        let mut routes = BTreeMap::new();
        for (key, spec) in parsed.routes {
            let (layout_name, dir) = match spec {
                RouteSpec::LayoutName(layout) => (layout, key.clone()),
                RouteSpec::Table {
                    layout,
                    project_dir,
                } => {
                    let dir = project_dir.unwrap_or_else(|| key.clone());
                    (layout, dir)
                }
            };
            if layout_name != "default" && !named.contains_key(&layout_name) {
                // A `[layouts.*]` that equalled default was dropped; treat
                // that name as the process default so a documentary alias
                // still loads.
                let named_as_default = parsed.layouts.get(&layout_name).is_some_and(|spec| {
                    expand_path(&spec.root).ok().is_some_and(|root| {
                        layout_key(&Layout::new(root, spec.prefix.clone())) == default_key
                    })
                });
                if !named_as_default {
                    return Err(
                        anyhow!("route {key:?} names unknown layout {layout_name:?}").into(),
                    );
                }
            }
            let store_as = if named.contains_key(&layout_name) {
                layout_name
            } else {
                "default".to_string()
            };
            routes.insert(key.to_lowercase(), (store_as, dir));
        }
        Ok(Self {
            default,
            named,
            routes,
        })
    }

    /// The process default, from `--root` / `VISSUE_ROOT` / cwd.
    #[must_use]
    pub fn default_layout(&self) -> &Layout {
        &self.default
    }

    /// Whether any route is configured.
    #[must_use]
    pub fn is_routed(&self) -> bool {
        !self.routes.is_empty()
    }

    /// Resolve a caller-facing project name to a layout and on-disk directory.
    #[must_use]
    pub fn route(&self, project: &str) -> ProjectRef {
        let key = project.to_lowercase();
        if let Some((layout_name, dir)) = self.routes.get(&key) {
            let layout = self.layout_named(layout_name).clone();
            return ProjectRef {
                layout,
                dir: dir.clone(),
                key: project.to_string(),
            };
        }
        ProjectRef {
            layout: self.default.clone(),
            dir: project.to_string(),
            key: project.to_string(),
        }
    }

    /// Unique layouts, `default` first, then named ones. Equality is
    /// `(canonical root, prefix)`.
    #[must_use]
    pub fn unique_layouts(&self) -> Vec<&Layout> {
        let mut out = Vec::with_capacity(1 + self.named.len());
        out.push(&self.default);
        let default_key = layout_key(&self.default);
        for layout in self.named.values() {
            if layout_key(layout) != default_key {
                out.push(layout);
            }
        }
        out
    }

    /// Projects an unscoped `list` / `projects` should show.
    ///
    /// Default-layout directories whose name is an identity route key are
    /// hidden. Alias keys appear; the alias directory on the routed layout
    /// does not appear under its raw name unless that name is also a default
    /// project.
    ///
    /// # Errors
    ///
    /// Returns an error if a projects directory cannot be read.
    pub fn visible_projects(&self) -> Result<Vec<ProjectRef>> {
        let identity_keys: Vec<String> = self
            .routes
            .iter()
            .filter(|(key, (_, dir))| key.as_str() == dir.to_lowercase())
            .map(|(key, _)| key.clone())
            .collect();
        let mut out = Vec::new();
        for name in store::list_projects(&self.default)? {
            if identity_keys.iter().any(|k| k == &name.to_lowercase()) {
                continue;
            }
            out.push(ProjectRef {
                layout: self.default.clone(),
                dir: name.clone(),
                key: name,
            });
        }
        for (key, (layout_name, dir)) in &self.routes {
            let layout = self.layout_named(layout_name).clone();
            out.push(ProjectRef {
                layout,
                dir: dir.clone(),
                key: key.clone(),
            });
        }
        out.sort_by_key(|a| a.key.to_lowercase());
        Ok(out)
    }

    /// Ids already used for `dir` on any unique layout, so a create can
    /// refuse a suffix the twin file already holds.
    ///
    /// # Errors
    ///
    /// Returns an error if a twin file exists and cannot be parsed.
    pub fn extra_ids_for(&self, dir: &str) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        for layout in self.unique_layouts() {
            let path = layout.project_issues_path(dir);
            let doc = IssueDoc::parse_file(dir, &path)?;
            ids.extend(doc.known_ids());
        }
        Ok(ids)
    }

    /// The twin files an id reservation has to read, rather than the ids in
    /// them.
    ///
    /// Reading the ids here and minting later is a read outside the lock that
    /// protects the write. Two creates for the same project in two roots each
    /// see the other's file before either has written, take different locks,
    /// because the locks are per file, and can mint the same suffix. A duplicate
    /// across layouts is not a cosmetic clash: `find_by_id` reports
    /// `DuplicateId` for it, so the issue becomes unreachable by id.
    ///
    /// So the caller hands over paths and the mint reads them under the same
    /// lock set it writes under.
    pub fn extra_id_paths_for(&self, dir: &str) -> Vec<PathBuf> {
        self.unique_layouts()
            .into_iter()
            .map(|layout| layout.project_issues_path(dir))
            .collect()
    }

    /// Locate one id. The longest matching route key or known project
    /// directory is tried first; then every remaining unique layout.
    ///
    /// # Errors
    ///
    /// [`Error::IssueNotFound`] when no layout has the id.
    /// [`Error::DuplicateId`] when two distinct layouts define it.
    pub fn find_by_id(&self, id: &str) -> Result<RouteHit> {
        let hint = self.hint_project(id);
        let mut hits: Vec<RouteHit> = Vec::new();
        let mut seen_keys = Vec::new();

        if let Some(name) = hint.as_deref() {
            let pref = self.route(name);
            let key = layout_key(&pref.layout);
            if let Some(hit) = lookup_in(&pref.layout, id)? {
                hits.push(hit);
            }
            seen_keys.push(key);
        }

        for layout in self.unique_layouts() {
            let key = layout_key(layout);
            if seen_keys.iter().any(|k| k == &key) {
                continue;
            }
            if let Some(hit) = lookup_in(layout, id)? {
                hits.push(hit);
            }
            seen_keys.push(key);
        }

        match hits.len() {
            0 => Err(Error::IssueNotFound { id: id.to_string() }),
            1 => Ok(hits.remove(0)),
            _ => Err(Error::DuplicateId {
                id: id.to_string(),
                paths: hits.into_iter().map(|h| h.path).collect(),
            }),
        }
    }

    /// Layouts a `backlinks` walk should scan for `id`.
    ///
    /// A known unique id answers on its own layout. An accession names a
    /// product rather than a heading, so it has no layout of its own and
    /// every tracker in reach is scanned. Only [`Error::IssueNotFound`]
    /// falls through to that walk: [`Error::DuplicateId`] and I/O stay
    /// errors, because an accession-shaped id that exists twice is a
    /// duplicate known issue, not a deed.
    ///
    /// # Errors
    ///
    /// [`Error::DuplicateId`] when two distinct layouts define the id.
    /// Any I/O error from [`Self::find_by_id`].
    pub fn layouts_for_backlinks(&self, id: &str) -> Result<Vec<Layout>> {
        match self.find_by_id(id) {
            Ok(hit) => Ok(vec![hit.layout]),
            Err(Error::IssueNotFound { .. }) if crate::ops::is_deed_accession(id) => {
                Ok(self.unique_layouts().into_iter().cloned().collect())
            }
            Err(err) => Err(err),
        }
    }

    fn hint_project(&self, id: &str) -> Option<String> {
        let mut names: Vec<String> = self.routes.keys().cloned().collect();
        for (_, dir) in self.routes.values() {
            names.push(dir.to_lowercase());
        }
        if let Ok(projects) = store::list_projects(&self.default) {
            names.extend(projects.into_iter().map(|p| p.to_lowercase()));
        }
        let mut best: Option<String> = None;
        for name in names {
            if name.is_empty() {
                continue;
            }
            if let Some(rest) = id.to_lowercase().strip_prefix(&name)
                && rest.starts_with('-')
                && rest.len() > 1
                && best.as_ref().is_none_or(|b| name.len() > b.len())
            {
                best = Some(name);
            }
        }
        best.or_else(|| {
            id.split_once('-')
                .map(|(head, _)| head.to_string())
                .filter(|h| !h.is_empty())
        })
    }

    fn layout_named(&self, name: &str) -> &Layout {
        if name == "default" {
            &self.default
        } else {
            self.named.get(name).unwrap_or(&self.default)
        }
    }

    /// Ids that appear under more than one distinct unique layout.
    ///
    /// # Errors
    ///
    /// Returns an error if a project file cannot be read.
    pub fn duplicate_ids(&self) -> Result<Vec<(String, Vec<PathBuf>)>> {
        let mut map: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for layout in self.unique_layouts() {
            for (project, heading) in store::load_all(layout)? {
                map.entry(heading.id)
                    .or_default()
                    .push(layout.project_issues_path(&project));
            }
        }
        Ok(map
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .collect())
    }
}

fn lookup_in(layout: &Layout, id: &str) -> Result<Option<RouteHit>> {
    match store::find_by_id(layout, id)? {
        Some((heading, path, project)) => Ok(Some(RouteHit {
            layout: layout.clone(),
            project,
            heading,
            path,
        })),
        None => Ok(None),
    }
}

fn layout_key(layout: &Layout) -> (PathBuf, String) {
    let root = layout
        .root()
        .canonicalize()
        .unwrap_or_else(|_| layout.root().to_path_buf());
    (root, layout.prefix().to_string())
}

fn routing_disabled() -> bool {
    match process_env::var("VISSUE_NO_ROUTE") {
        Ok(v) => {
            let t = v.trim();
            !t.is_empty() && t != "0" && !t.eq_ignore_ascii_case("false")
        }
        Err(_) => false,
    }
}

fn user_config_path() -> Option<PathBuf> {
    if let Ok(raw) = process_env::var("VISSUE_CONFIG") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    let base = process_env::var("XDG_CONFIG_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            process_env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("vissue/config.toml"))
}

fn expand_path(raw: &str) -> Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("layout root is empty").into());
    }
    let with_home = if trimmed == "~" {
        home_dir()?
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        format!("{}/{}", home_dir()?, rest)
    } else {
        trimmed.to_string()
    };
    let expanded = expand_vars(&with_home)?;
    let path = PathBuf::from(&expanded);
    if !path.is_absolute() {
        return Err(anyhow!(
            "layout root must be absolute after ~ and environment expansion, got {raw:?}"
        )
        .into());
    }
    Ok(path)
}

fn home_dir() -> Result<String> {
    process_env::var("HOME")
        .or_else(|_| process_env::var("USERPROFILE"))
        .map_err(|_| anyhow!("~ in a layout root requires HOME"))
        .map_err(Error::from)
}

fn expand_vars(input: &str) -> Result<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                if let Some(rel) = chars[i + 2..].iter().position(|&c| c == '}') {
                    let name: String = chars[i + 2..i + 2 + rel].iter().collect();
                    out.push_str(&lookup_var(&name)?);
                    i = i + 3 + rel;
                    continue;
                }
            } else {
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_')
                {
                    end += 1;
                }
                if end > start {
                    let name: String = chars[start..end].iter().collect();
                    out.push_str(&lookup_var(&name)?);
                    i = end;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    Ok(out)
}

fn lookup_var(name: &str) -> Result<String> {
    process_env::var(name)
        .map_err(|_| anyhow!("environment variable {name} is unset in layout root"))
        .map_err(Error::from)
}

#[cfg(test)]
#[allow(deprecated_safe_2024)]
mod tests {
    use super::*;
    use std::fs;

    fn seed(layout: &Layout, project: &str, id: &str, title: &str) {
        let path = layout.project_issues_path(project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!("* TODO {title}\n:PROPERTIES:\n:ID:         {id}\n:END:\n"),
        )
        .unwrap();
    }

    fn write_cfg(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("config.toml");
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn missing_config_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = Layout::new(tmp.path(), "Software");
        let router = Router::from_file(layout.clone(), &tmp.path().join("absent.toml"));
        // from_file requires the file; load() is the missing-path no-op.
        assert!(router.is_err());
        let router = Router::unrouted(layout);
        assert!(!router.is_routed());
        let pref = router.route("surf");
        assert_eq!(pref.dir, "surf");
        assert_eq!(pref.layout, router.default_layout().clone());
    }

    #[test]
    fn a_named_route_wins_over_the_default_root() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let work = tmp.path().join("work");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&work).unwrap();
        seed(&Layout::new(&work, "Issues"), "surf", "surf-abcd", "routed");
        seed(
            &Layout::new(&vault, "Software"),
            "surf",
            "surf-old1",
            "historical",
        );
        let cfg = write_cfg(
            tmp.path(),
            &format!(
                "[layouts.work]\nroot = \"{}\"\nprefix = \"Issues\"\n\n[routes]\nsurf = \"work\"\n",
                work.display()
            ),
        );
        let router = Router::from_file(Layout::new(&vault, "Software"), &cfg).unwrap();
        let pref = router.route("surf");
        assert_eq!(pref.dir, "surf");
        assert_eq!(pref.layout.prefix(), "Issues");
        assert_eq!(pref.layout.root(), work.as_path());
        let hit = router.find_by_id("surf-abcd").unwrap();
        assert_eq!(hit.heading.title, "routed");
        let old = router.find_by_id("surf-old1").unwrap();
        assert_eq!(old.heading.title, "historical");
    }

    #[test]
    fn an_alias_writes_an_existing_directory_without_renaming_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let work = tmp.path().join("work");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&work).unwrap();
        seed(
            &Layout::new(&work, "Issues"),
            "solver",
            "solver-aaaa",
            "site ticket",
        );
        seed(
            &Layout::new(&vault, "Software"),
            "solver",
            "solver-bbbb",
            "product ticket",
        );
        let cfg = write_cfg(
            tmp.path(),
            &format!(
                "[layouts.work]\nroot = \"{}\"\nprefix = \"Issues\"\n\n[routes.solver-work]\nlayout = \"work\"\nproject_dir = \"solver\"\n",
                work.display()
            ),
        );
        let router = Router::from_file(Layout::new(&vault, "Software"), &cfg).unwrap();
        let pref = router.route("solver-work");
        assert_eq!(pref.dir, "solver");
        assert_eq!(pref.layout.prefix(), "Issues");
        let raw = router.route("solver");
        assert_eq!(raw.layout.prefix(), "Software");
        assert_eq!(raw.dir, "solver");
        let names: Vec<_> = router
            .visible_projects()
            .unwrap()
            .into_iter()
            .map(|p| p.key)
            .collect();
        assert!(names.iter().any(|n| n == "solver"), "{names:?}");
        assert!(names.iter().any(|n| n == "solver-work"), "{names:?}");
        let hit = router.find_by_id("solver-aaaa").unwrap();
        assert_eq!(hit.layout.prefix(), "Issues");
    }

    #[test]
    fn a_layout_that_equals_default_is_not_scanned_twice() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        seed(
            &Layout::new(&vault, "Software"),
            "only",
            "only-zzzz",
            "once",
        );
        let cfg = write_cfg(
            tmp.path(),
            &format!(
                "[layouts.vault]\nroot = \"{}\"\nprefix = \"Software\"\n",
                vault.display()
            ),
        );
        let router = Router::from_file(Layout::new(&vault, "Software"), &cfg).unwrap();
        assert_eq!(router.unique_layouts().len(), 1);
        let hit = router.find_by_id("only-zzzz").unwrap();
        assert_eq!(hit.heading.title, "once");
    }

    #[test]
    fn duplicate_id_on_two_distinct_layouts_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let work = tmp.path().join("work");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&work).unwrap();
        seed(&Layout::new(&work, "Issues"), "surf", "surf-same", "a");
        seed(&Layout::new(&vault, "Software"), "surf", "surf-same", "b");
        let cfg = write_cfg(
            tmp.path(),
            &format!(
                "[layouts.work]\nroot = \"{}\"\nprefix = \"Issues\"\n",
                work.display()
            ),
        );
        let router = Router::from_file(Layout::new(&vault, "Software"), &cfg).unwrap();
        let err = router.find_by_id("surf-same").unwrap_err();
        match err {
            Error::DuplicateId { id, paths } => {
                assert_eq!(id, "surf-same");
                assert_eq!(paths.len(), 2);
            }
            other => panic!("expected DuplicateId, got {other}"),
        }
    }

    /// An accession-shaped heading that exists twice is a duplicate known
    /// issue. Swallowing that error would scan every layout as a deed and
    /// answer with a cites list.
    #[test]
    fn an_accession_shaped_duplicate_id_is_not_walked_as_a_deed() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let work = tmp.path().join("work");
        let extra = tmp.path().join("extra");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::create_dir_all(&extra).unwrap();
        seed(
            &Layout::new(&work, "Issues"),
            "deed",
            "deed-patch-overlay",
            "work copy",
        );
        seed(
            &Layout::new(&vault, "Software"),
            "deed",
            "deed-patch-overlay",
            "vault copy",
        );
        let cite = Layout::new(&extra, "Software").project_issues_path("atlas");
        fs::create_dir_all(cite.parent().unwrap()).unwrap();
        fs::write(
            &cite,
            "* TODO cites it\n:PROPERTIES:\n:ID:         atlas-cite1\n:DEEDS:      deed-patch-overlay\n:END:\n",
        )
        .unwrap();
        let cfg = write_cfg(
            tmp.path(),
            &format!(
                "[layouts.work]\nroot = \"{}\"\nprefix = \"Issues\"\n\n[layouts.extra]\nroot = \"{}\"\nprefix = \"Software\"\n",
                work.display(),
                extra.display()
            ),
        );
        let router = Router::from_file(Layout::new(&vault, "Software"), &cfg).unwrap();
        let err = router
            .layouts_for_backlinks("deed-patch-overlay")
            .unwrap_err();
        match err {
            Error::DuplicateId { id, paths } => {
                assert_eq!(id, "deed-patch-overlay");
                assert_eq!(paths.len(), 2);
            }
            other => panic!("expected DuplicateId, not a cites walk, got {other}"),
        }
    }

    #[test]
    fn an_unused_accession_still_scans_every_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let work = tmp.path().join("work");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&work).unwrap();
        let cfg = write_cfg(
            tmp.path(),
            &format!(
                "[layouts.work]\nroot = \"{}\"\nprefix = \"Issues\"\n",
                work.display()
            ),
        );
        let router = Router::from_file(Layout::new(&vault, "Software"), &cfg).unwrap();
        let layouts = router.layouts_for_backlinks("deed-quote-unused").unwrap();
        assert_eq!(layouts.len(), 2, "missing accession scans every tracker");
    }

    #[test]
    fn extra_ids_union_the_twin_file() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let work = tmp.path().join("work");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&work).unwrap();
        seed(&Layout::new(&vault, "Software"), "surf", "surf-old1", "old");
        let cfg = write_cfg(
            tmp.path(),
            &format!(
                "[layouts.work]\nroot = \"{}\"\nprefix = \"Issues\"\n\n[routes]\nsurf = \"work\"\n",
                work.display()
            ),
        );
        let router = Router::from_file(Layout::new(&vault, "Software"), &cfg).unwrap();
        let ids = router.extra_ids_for("surf").unwrap();
        assert!(ids.iter().any(|i| i == "surf-old1"), "{ids:?}");
    }

    #[test]
    fn relative_roots_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = write_cfg(
            tmp.path(),
            "[layouts.work]\nroot = \"relative/path\"\nprefix = \"Issues\"\n",
        );
        let err = Router::from_file(Layout::new(tmp.path(), "Software"), &cfg).unwrap_err();
        assert!(err.to_string().contains("absolute"), "{err}");
    }

    #[test]
    fn tilde_and_env_expand_in_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        fs::create_dir_all(&work).unwrap();
        crate::process_env::override_var("HOME", Some(tmp.path().to_str().unwrap()));
        crate::process_env::override_var("WORKROOT", Some(work.to_str().unwrap()));
        let cfg_home = write_cfg(
            tmp.path(),
            "[layouts.work]\nroot = \"~/work\"\nprefix = \"Issues\"\n",
        );
        let router = Router::from_file(Layout::new(tmp.path(), "Software"), &cfg_home).unwrap();
        assert_eq!(router.route("x").layout.prefix(), "Software");
        assert_eq!(
            router.named.get("work").map(|l| l.root().to_path_buf()),
            Some(work.clone())
        );
        let cfg_env = write_cfg(
            tmp.path(),
            "[layouts.work]\nroot = \"$WORKROOT\"\nprefix = \"Issues\"\n",
        );
        let router = Router::from_file(Layout::new(tmp.path(), "Software"), &cfg_env).unwrap();
        assert_eq!(
            router.named.get("work").map(|l| l.root().to_path_buf()),
            Some(work)
        );
        crate::process_env::clear_override("HOME");
        crate::process_env::clear_override("WORKROOT");
    }

    #[test]
    fn an_unknown_route_layout_is_a_load_error() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = write_cfg(tmp.path(), "[routes]\nsurf = \"missing\"\n");
        let err = Router::from_file(Layout::new(tmp.path(), "Software"), &cfg).unwrap_err();
        assert!(err.to_string().contains("unknown layout"), "{err}");
    }

    #[test]
    fn no_route_env_disables_the_table() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        fs::create_dir_all(&work).unwrap();
        let cfg = write_cfg(
            tmp.path(),
            &format!(
                "[layouts.work]\nroot = \"{}\"\nprefix = \"Issues\"\n\n[routes]\nsurf = \"work\"\n",
                work.display()
            ),
        );
        crate::process_env::override_var("VISSUE_NO_ROUTE", Some("1"));
        crate::process_env::override_var("VISSUE_CONFIG", Some(cfg.to_str().unwrap()));
        let router = Router::load(Layout::new(tmp.path(), "Software")).unwrap();
        assert!(!router.is_routed());
        assert_eq!(router.route("surf").layout.prefix(), "Software");
        crate::process_env::clear_override("VISSUE_NO_ROUTE");
        crate::process_env::clear_override("VISSUE_CONFIG");
    }
}
