//! `create` and `q` choose a project in a fixed order.
//!
//! The order is the flag, a config route, then the repository name when it
//! matches a project. Otherwise the projects are listed and nothing is filed.

#![allow(missing_docs)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn vissue_at(dir: &Path, root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vissue"))
        .env("VISSUE_NO_ROUTE", "1")
        .env_remove("VISSUE_ROOT")
        .env_remove("ISSUE_ROOT")
        .env_remove("VISSUE_PREFIX")
        .env_remove("VISSUE_CONFIG")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .current_dir(dir)
        .args(["--root", root.to_str().unwrap()])
        .args(args)
        .output()
        .expect("run vissue")
}

fn vissue_routed(dir: &Path, root: &Path, config: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vissue"))
        .env_remove("VISSUE_NO_ROUTE")
        .env_remove("VISSUE_ROOT")
        .env_remove("ISSUE_ROOT")
        .env_remove("VISSUE_PREFIX")
        .env("VISSUE_CONFIG", config)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .current_dir(dir)
        .args(["--root", root.to_str().unwrap()])
        .args(args)
        .output()
        .expect("run vissue")
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
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
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn id_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn tracker() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("Software")).unwrap();
    dir
}

fn seed(tracker: &Path, project: &str) {
    let created = vissue_at(
        tracker,
        tracker,
        &["create", "-p", project, "--quiet", "seed"],
    );
    assert!(created.status.success(), "{}", text(&created));
    assert!(
        project_line(&created).contains(&format!("project: {project}")),
        "{}",
        text(&created)
    );
}

fn project_line(out: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Whether `dir` lists an entry spelled exactly `name`. A file system that
/// folds case, the macOS default, finds `Software/parser` when only
/// `Software/Parser` is there.
fn listed(dir: &Path, name: &str) -> bool {
    fs::read_dir(dir)
        .unwrap()
        .any(|e| e.unwrap().file_name() == name)
}

fn assert_no_prefix_project(root: &Path) {
    assert!(
        !root.join("Software/Software").exists(),
        "a project directory was created for the prefix"
    );
}

/// `-p` is first. A repository name does not override it, and the project
/// used is printed. Quiet stdout stays the id.
#[test]
fn a_flag_names_the_project_and_is_printed() {
    let tracker = tracker();
    let work = tempfile::tempdir().unwrap();
    let repo = work.path().join("parser");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);

    for cmd in [
        &["create", "-p", "atlas", "--quiet", "named"][..],
        &["q", "-p", "atlas", "named again"][..],
    ] {
        let created = vissue_at(&repo, tracker.path(), cmd);
        assert!(created.status.success(), "{}", text(&created));
        assert!(id_of(&created).starts_with("atlas-"), "{}", text(&created));
        assert!(
            String::from_utf8_lossy(&created.stderr).contains("project: atlas"),
            "{}",
            text(&created)
        );
        assert!(
            !id_of(&created).contains("project:"),
            "quiet stdout gained a second line: {}",
            id_of(&created)
        );
    }
    assert!(!tracker.path().join("Software/parser").exists());
    assert_no_prefix_project(tracker.path());

    let loud = vissue_at(
        &repo,
        tracker.path(),
        &["create", "-p", "atlas", "said aloud"],
    );
    assert!(loud.status.success(), "{}", text(&loud));
    assert!(
        String::from_utf8_lossy(&loud.stdout).starts_with("project: atlas\n"),
        "{}",
        text(&loud)
    );
}

/// Standing inside `<root>/<prefix>/<name>/` is the config route, ahead of
/// the repository that contains the tracker.
#[test]
fn standing_in_a_project_directory_is_the_config_route() {
    let tracker = tracker();
    let root = tracker.path();
    let inside = root.join("Software/kept");
    fs::create_dir_all(&inside).unwrap();
    git(root, &["init", "-q"]);
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/parser.git",
        ],
    );
    fs::write(
        root.join(".project-ctx.toml"),
        "[project]\nname = \"fromfile\"\n",
    )
    .unwrap();

    for cmd in [
        &["create", "--quiet", "where I stand"][..],
        &["q", "where I stand again"][..],
    ] {
        let created = vissue_at(&inside, root, cmd);
        assert!(created.status.success(), "{}", text(&created));
        assert!(id_of(&created).starts_with("kept-"), "{}", text(&created));
        assert!(
            String::from_utf8_lossy(&created.stderr).contains("project: kept"),
            "{}",
            text(&created)
        );
    }
    assert!(root.join("Software/kept/issues.org").is_file());
    assert!(!root.join("Software/parser").exists());
    assert!(!root.join("Software/fromfile").exists());
    assert_no_prefix_project(root);
}

/// A context file that names an existing project is the config route, ahead
/// of the repository. One that names nothing does not create a directory.
#[test]
fn a_context_file_is_a_config_route_only_when_it_names_a_project() {
    let tracker = tracker();
    seed(tracker.path(), "fromfile");
    seed(tracker.path(), "parser");
    let work = tempfile::tempdir().unwrap();
    let repo = work.path().join("parser");
    let nested = repo.join("src");
    fs::create_dir_all(&nested).unwrap();
    git(&repo, &["init", "-q"]);
    fs::write(
        repo.join(".project-ctx.toml"),
        "[project]\nname = \"fromfile\"\n",
    )
    .unwrap();

    for cmd in [
        &["create", "--quiet", "from the context file"][..],
        &["q", "again from the file"][..],
    ] {
        let created = vissue_at(&nested, tracker.path(), cmd);
        assert!(created.status.success(), "{}", text(&created));
        assert!(
            id_of(&created).starts_with("fromfile-"),
            "{}",
            text(&created)
        );
        assert!(
            String::from_utf8_lossy(&created.stderr).contains("project: fromfile"),
            "{}",
            text(&created)
        );
    }

    fs::write(
        repo.join(".project-ctx.toml"),
        "[project]\nname = \"missing\"\n",
    )
    .unwrap();
    let created = vissue_at(
        &nested,
        tracker.path(),
        &["create", "--quiet", "not that name"],
    );
    assert!(created.status.success(), "{}", text(&created));
    assert!(id_of(&created).starts_with("parser-"), "{}", text(&created));
    assert!(
        String::from_utf8_lossy(&created.stderr).contains("project: parser"),
        "{}",
        text(&created)
    );
    assert!(!tracker.path().join("Software/missing").exists());
    assert_no_prefix_project(tracker.path());
}

/// Outside a repository, a context file that names an existing project is
/// the config route. A name that is not a project creates nothing.
#[test]
fn a_context_file_outside_a_repo_names_an_existing_project() {
    let tracker = tracker();
    seed(tracker.path(), "atlas");
    let workspace = tempfile::tempdir().unwrap();
    fs::write(
        workspace.path().join(".project-ctx.toml"),
        "[project]\nname = \"atlas\"\n",
    )
    .unwrap();
    let notes = workspace.path().join("notes");
    fs::create_dir_all(notes.join(".git")).unwrap();

    let created = vissue_at(&notes, tracker.path(), &["create", "--quiet", "no repo"]);
    assert!(created.status.success(), "{}", text(&created));
    assert!(id_of(&created).starts_with("atlas-"), "{}", text(&created));
    assert!(
        String::from_utf8_lossy(&created.stderr).contains("project: atlas"),
        "{}",
        text(&created)
    );
    assert_no_prefix_project(tracker.path());

    fs::write(
        workspace.path().join(".project-ctx.toml"),
        "[project]\nname = \"missing\"\n",
    )
    .unwrap();
    let missed = vissue_at(&notes, tracker.path(), &["q", "no such project"]);
    assert!(!missed.status.success(), "{}", text(&missed));
    assert!(text(&missed).contains("projects:"), "{}", text(&missed));
    assert!(text(&missed).contains("atlas"), "{}", text(&missed));
    assert!(!tracker.path().join("Software/missing").exists());
}

/// A route whose key is the repository name is the config route. The issue
/// is written on the routed layout, and the default layout gains no directory.
#[test]
fn a_user_config_route_for_the_repository_is_the_config_route() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    let work = tmp.path().join("work");
    fs::create_dir_all(vault.join("Software")).unwrap();
    fs::create_dir_all(work.join("Issues")).unwrap();
    let cfg = tmp.path().join("config.toml");
    fs::write(
        &cfg,
        format!(
            "[layouts.work]\nroot = \"{}\"\nprefix = \"Issues\"\n\n[routes]\nparser = \"work\"\n",
            work.display()
        ),
    )
    .unwrap();
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

    for cmd in [
        &["create", "--quiet", "routed"][..],
        &["q", "routed again"][..],
    ] {
        let created = vissue_routed(&repo, &vault, &cfg, cmd);
        assert!(created.status.success(), "{}", text(&created));
        assert!(id_of(&created).starts_with("parser-"), "{}", text(&created));
        assert!(
            String::from_utf8_lossy(&created.stderr).contains("project: parser"),
            "{}",
            text(&created)
        );
    }
    assert!(work.join("Issues/parser/issues.org").is_file());
    // The twin-id lock may create the empty directory on the default layout.
    // The issue itself is not filed there.
    assert!(!vault.join("Software/parser/issues.org").exists());
    assert!(!vault.join("Software/Software").exists());
}

/// The repository name is the project when that project already exists, for
/// `create` and for `q`. It is not used to create the directory.
#[test]
fn the_repository_name_is_used_when_it_matches_a_project() {
    let root = tracker();
    seed(root.path(), "parser");
    let work = tempfile::tempdir().unwrap();
    let repo = work.path().join("parser");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);

    for cmd in [
        &["create", "--quiet", "from the repo"][..],
        &["q", "quick one"][..],
    ] {
        let created = vissue_at(&repo, root.path(), cmd);
        assert!(created.status.success(), "{}", text(&created));
        assert!(id_of(&created).starts_with("parser-"), "{}", text(&created));
        assert!(
            String::from_utf8_lossy(&created.stderr).contains("project: parser"),
            "{}",
            text(&created)
        );
    }
    assert_no_prefix_project(root.path());

    let fresh = tracker();
    let other = work.path().join("brandnew");
    fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q"]);
    seed(fresh.path(), "atlas");
    let missed = vissue_at(&other, fresh.path(), &["create", "--quiet", "no match"]);
    assert!(!missed.status.success(), "{}", text(&missed));
    let err = text(&missed);
    assert!(err.contains("projects:"), "{err}");
    assert!(err.contains("atlas"), "{err}");
    assert!(err.contains("brandnew"), "{err}");
    assert!(!fresh.path().join("Software/brandnew").exists());
    assert_no_prefix_project(fresh.path());
}

/// The checkout directory can be named anything. `origin` is the repository,
/// and it matches an existing project by case.
#[test]
fn a_remote_name_matches_an_existing_project_by_case() {
    let tracker = tracker();
    seed(tracker.path(), "Parser");
    let work = tempfile::tempdir().unwrap();
    let repo = work.path().join("checkout");
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

    for cmd in [
        &["create", "--quiet", "via origin"][..],
        &["q", "also via origin"][..],
    ] {
        let created = vissue_at(&repo, tracker.path(), cmd);
        assert!(created.status.success(), "{}", text(&created));
        assert!(id_of(&created).starts_with("Parser-"), "{}", text(&created));
        assert!(
            String::from_utf8_lossy(&created.stderr).contains("project: Parser"),
            "{}",
            text(&created)
        );
    }
    assert!(!listed(&tracker.path().join("Software"), "parser"));
    assert!(!tracker.path().join("Software/checkout").exists());
    assert_no_prefix_project(tracker.path());
}

/// A `.project-ctx.toml` above the clone names Software. That file is not a
/// config route. The repository is used when it matches, and the prefix is
/// not created when it does not.
#[test]
fn a_context_file_above_a_repo_does_not_select_the_prefix() {
    let root = tracker();
    seed(root.path(), "parser");
    let workspace = tempfile::tempdir().unwrap();
    fs::write(
        workspace.path().join(".project-ctx.toml"),
        "[project]\nname = \"Software\"\n",
    )
    .unwrap();
    let repo = workspace.path().join("checkout");
    let nested = repo.join("src");
    fs::create_dir_all(&nested).unwrap();
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

    for cmd in [
        &["create", "--quiet", "from the clone"][..],
        &["q", "quick from the clone"][..],
    ] {
        let created = vissue_at(&nested, root.path(), cmd);
        assert!(created.status.success(), "{}", text(&created));
        assert!(id_of(&created).starts_with("parser-"), "{}", text(&created));
        assert!(
            String::from_utf8_lossy(&created.stderr).contains("project: parser"),
            "{}",
            text(&created)
        );
    }
    assert_no_prefix_project(root.path());

    let bare = tracker();
    seed(bare.path(), "atlas");
    let folder = workspace.path().join("plain-folder");
    fs::create_dir_all(&folder).unwrap();
    git(&folder, &["init", "-q"]);
    let missed = vissue_at(&folder, bare.path(), &["q", "from the folder"]);
    assert!(!missed.status.success(), "{}", text(&missed));
    let err = text(&missed);
    assert!(err.contains("projects:"), "{err}");
    assert!(err.contains("atlas"), "{err}");
    assert!(!bare.path().join("Software/plain-folder").exists());
    assert!(!bare.path().join("Software/Software").exists());
}

/// No flag, no config route, and no repository lists the projects and
/// creates nothing, including the prefix.
#[test]
fn no_config_route_and_no_repository_lists_projects() {
    let tracker = tracker();
    seed(tracker.path(), "atlas");
    seed(tracker.path(), "beacon");
    let work = tempfile::tempdir().unwrap();
    let plain = work.path().join("plain");
    fs::create_dir_all(plain.join(".git")).unwrap();

    for cmd in [
        &["create", "--quiet", "nowhere"][..],
        &["q", "nowhere either"][..],
    ] {
        let created = vissue_at(&plain, tracker.path(), cmd);
        assert!(!created.status.success(), "{}", text(&created));
        let err = text(&created);
        assert!(err.contains("projects:"), "{err}");
        assert!(err.contains("atlas"), "{err}");
        assert!(err.contains("beacon"), "{err}");
        assert!(!err.contains("project:"), "{err}");
    }
    assert_no_prefix_project(tracker.path());
}

/// `fold` still stops after the context file.
#[test]
fn fold_inside_a_repo_still_asks_for_a_project() {
    let tracker = tracker();
    let work = tempfile::tempdir().unwrap();
    let repo = work.path().join("parser");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);

    let folded = vissue_at(&repo, tracker.path(), &["fold", "inbox.org"]);
    assert!(!folded.status.success(), "{}", text(&folded));
    assert!(
        text(&folded).contains(".project-ctx.toml"),
        "{}",
        text(&folded)
    );
    assert!(!tracker.path().join("Software/parser").exists());
    assert_no_prefix_project(tracker.path());
}

#[test]
fn help_states_the_order() {
    for cmd in ["create", "q"] {
        let out = Command::new(env!("CARGO_BIN_EXE_vissue"))
            .args([cmd, "--help"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{cmd}");
        let help = String::from_utf8(out.stdout).unwrap();
        assert!(help.contains("config route"), "{cmd}:\n{help}");
        assert!(help.contains(".project-ctx.toml"), "{cmd}:\n{help}");
        assert!(help.contains("matches a project"), "{cmd}:\n{help}");
        assert!(help.contains("projects are listed"), "{cmd}:\n{help}");
        assert!(help.contains("printed"), "{cmd}:\n{help}");
        assert!(
            !help.contains("prefix default"),
            "{cmd} help still falls back to the prefix:\n{help}"
        );
    }
    let fold = Command::new(env!("CARGO_BIN_EXE_vissue"))
        .args(["fold", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8(fold.stdout).unwrap();
    assert!(help.contains(".project-ctx.toml"), "{help}");
    assert!(
        !help.contains("matches a project"),
        "fold help grew the create order:\n{help}"
    );
}
