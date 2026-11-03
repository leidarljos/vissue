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
    /// Which tracker the seat means when nobody names one. Read by
    /// `Layout::resolve` rather than here, and declared so that naming it does
    /// not make the file unreadable to the router.
    root: Option<String>,
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

    /// A `[layouts.*]` entry by name, or the default for `default`; `None`
    /// when the user's config names no such layout.
    #[must_use]
    pub fn named_layout(&self, name: &str) -> Option<&Layout> {
        if name == "default" {
            return Some(&self.default);
        }
        self.named.get(name)
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

    /// The project `create` and `q` write into.
    ///
    /// The order, first match:
    ///
    /// 1. The flag (`explicit`), including a name that has no directory yet.
    ///    Naming a project is how a tracker grows; that directory is asked
    ///    for, not invented.
    /// 2. A config route for `cwd`. That is the project directory under the
    ///    default prefix the caller stands in, then `[project].name` in a
    ///    `.project-ctx.toml` when the name is already a project or a route
    ///    (inside a git repository the walk stops at the top-level
    ///    directory), then a route whose key is the repository name. A route
    ///    is configuration the seat wrote, so the first issue there creates
    ///    the directory on the layout the route names.
    /// 3. The repository name when it matches a project that already exists.
    ///    The name is the `origin` remote's repository, or the top-level
    ///    directory when the repository has no remote.
    /// 4. An error listing the projects a seat can pass with `--project`.
    ///
    /// Steps 2 and 3 never create a project directory for a name that is not
    /// already a project or a route. The prefix is not a project.
    ///
    /// # Errors
    ///
    /// Returns an error when the flag is empty, a name matches two projects
    /// by case, the projects directory cannot be read, or nothing in the
    /// order names a project.
    pub fn choose_project(&self, explicit: Option<&str>) -> Result<String> {
        let cwd = std::env::current_dir().context("resolve current directory as a project")?;
        self.choose_project_at(explicit, &cwd)
    }

    /// [`Self::choose_project`] for a caller that already knows `cwd`.
    ///
    /// # Errors
    ///
    /// Same as [`Self::choose_project`].
    pub(crate) fn choose_project_at(&self, explicit: Option<&str>, cwd: &Path) -> Result<String> {
        if let Some(name) = explicit {
            if name.is_empty() {
                return Err(anyhow!("--project given but empty").into());
            }
            return self.accept_flag(name);
        }
        let top = crate::config::repo_toplevel(cwd);
        let repo = top.as_deref().and_then(crate::config::repository_name);
        if let Some(name) = self.config_route(cwd, top.as_deref(), repo.as_deref())? {
            return Ok(name);
        }
        if let Some(name) = repo.as_deref()
            && let Some(matched) = crate::config::matching_project(self.default_layout(), name)?
        {
            return Ok(matched);
        }
        Err(self.unresolved_project(repo.as_deref()).into())
    }

    /// A flag names its project. A route key is returned as given so
    /// [`Self::route`] still applies; any other name case-folds onto the
    /// directory that already exists.
    fn accept_flag(&self, name: &str) -> Result<String> {
        if self.route_key(name).is_some() {
            return Ok(name.to_string());
        }
        let pref = self.route(name);
        store::resolve_existing_project_case(&pref.layout, &pref.dir)
    }

    /// Config route for `cwd`, if one names a project or a route.
    fn config_route(
        &self,
        cwd: &Path,
        repo_top: Option<&Path>,
        repo_name: Option<&str>,
    ) -> Result<Option<String>> {
        if let Some(name) =
            crate::config::project_from_tracker_path(&self.default.projects_dir(), cwd)
        {
            return Ok(Some(self.pass_directory_name(name)?));
        }
        if let Some(name) = crate::config::context_project(cwd, repo_top)
            && let Some(chosen) = self.route_or_existing(&name)?
        {
            return Ok(Some(chosen));
        }
        if let Some(name) = repo_name
            && let Some(key) = self.route_key(name)
        {
            return Ok(Some(key));
        }
        Ok(None)
    }

    /// The on-disk directory the seat is standing in. A route for that name
    /// still wins when the issue is written, so the name is passed through.
    fn pass_directory_name(&self, name: String) -> Result<String> {
        if self.route_key(&name).is_some() {
            return Ok(name);
        }
        store::resolve_existing_project_case(&self.default, &name)
    }

    /// A route key, or an existing project on the default layout.
    fn route_or_existing(&self, name: &str) -> Result<Option<String>> {
        if let Some(key) = self.route_key(name) {
            return Ok(Some(key));
        }
        crate::config::matching_project(&self.default, name)
    }

    /// The stored route key for `name`, compared without regard to case.
    fn route_key(&self, name: &str) -> Option<String> {
        let key = name.to_lowercase();
        self.routes.contains_key(&key).then_some(key)
    }

    /// What `create` prints when the order names nothing.
    ///
    /// The list is the same names `projects` prints, so the seat can pass
    /// one of them with `--project`.
    fn unresolved_project(&self, repo: Option<&str>) -> anyhow::Error {
        let listed = match self.visible_projects() {
            Ok(projects) => {
                let names: Vec<String> = projects.into_iter().map(|project| project.key).collect();
                if names.is_empty() {
                    "(none)".to_string()
                } else {
                    names.join(", ")
                }
            }
            Err(err) => format!("(unreadable: {err})"),
        };
        match repo {
            Some(name) => anyhow!(
                "no project chosen: repository {name} matches no project and no config route names one. projects: {listed}"
            ),
            None => anyhow!(
                "no project chosen: no config route names one and no repository name matches a project. projects: {listed}"
            ),
        }
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

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .args([
                "-c",
                "user.email=dev@example.invalid",
                "-c",
                "user.name=dev",
                "-c",
                "init.defaultBranch=main",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A route whose key is the repository name is the config route, before
    /// the repository is asked to match a directory on the default layout.
    /// The default layout gains no project directory from that choice.
    #[test]
    fn a_route_for_the_repository_is_chosen_before_a_directory_is_created() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let work = tmp.path().join("work");
        fs::create_dir_all(vault.join("Software")).unwrap();
        fs::create_dir_all(&work).unwrap();
        let repo = tmp.path().join("checkout");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/acme/parser.git",
            ],
        );
        let cfg = write_cfg(
            tmp.path(),
            &format!(
                "[layouts.work]\nroot = \"{}\"\nprefix = \"Issues\"\n\n[routes]\nparser = \"work\"\n",
                work.display()
            ),
        );
        let router = Router::from_file(Layout::new(&vault, "Software"), &cfg).unwrap();
        assert_eq!(router.choose_project_at(None, &repo).unwrap(), "parser");
        assert!(!vault.join("Software/parser").exists());
        assert!(!vault.join("Software/Software").exists());

        // An empty `.git` is not a repository, and discovery stops there.
        let plain = tmp.path().join("plain");
        fs::create_dir_all(plain.join(".git")).unwrap();
        let listed = router.choose_project_at(None, &plain).unwrap_err();
        assert!(
            listed.to_string().contains("projects:"),
            "no route and no repository lists projects: {listed}"
        );
        assert!(listed.to_string().contains("parser"), "{listed}");
    }
}
