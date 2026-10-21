//! `create` and `q` with no `-p` take the project from the repository.

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

/// A create inside a repository was filed under the prefix. The repository
/// name is the project, for `create` and for `q`.
#[test]
fn create_and_q_inside_a_repo_file_under_that_repo() {
    let tracker = tracker();
    let work = tempfile::tempdir().unwrap();
    let repo = work.path().join("parser");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);

    let created = vissue_at(
        &repo,
        tracker.path(),
        &["create", "--quiet", "from the repo"],
    );
    assert!(created.status.success(), "{}", text(&created));
    let id = id_of(&created);
    assert!(id.starts_with("parser-"), "{id}");
    let filed = tracker.path().join("Software/parser/issues.org");
    let board = fs::read_to_string(&filed).unwrap();
    assert!(board.contains("from the repo"), "{board}");
    assert!(
        !tracker.path().join("Software/Software/issues.org").exists(),
        "the issue was filed under the prefix"
    );

    let quick = vissue_at(&repo, tracker.path(), &["q", "quick one"]);
    assert!(quick.status.success(), "{}", text(&quick));
    assert!(id_of(&quick).starts_with("parser-"), "{}", text(&quick));
}

/// The checkout directory can be named anything. `origin` is the repository.
#[test]
fn a_remote_name_beats_the_checkout_directory() {
    let tracker = tracker();
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

    let created = vissue_at(&repo, tracker.path(), &["create", "--quiet", "via origin"]);
    assert!(created.status.success(), "{}", text(&created));
    assert!(id_of(&created).starts_with("parser-"), "{}", text(&created));
    assert!(tracker.path().join("Software/parser/issues.org").is_file());
    assert!(!tracker.path().join("Software/checkout/issues.org").exists());

    let quick = vissue_at(&repo, tracker.path(), &["q", "also via origin"]);
    assert!(quick.status.success(), "{}", text(&quick));
    assert!(id_of(&quick).starts_with("parser-"), "{}", text(&quick));
}

/// `-p` is first. A repository name does not override it.
#[test]
fn an_explicit_project_beats_the_repo() {
    let tracker = tracker();
    let work = tempfile::tempdir().unwrap();
    let repo = work.path().join("parser");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);

    let created = vissue_at(
        &repo,
        tracker.path(),
        &["create", "-p", "atlas", "--quiet", "named"],
    );
    assert!(created.status.success(), "{}", text(&created));
    assert!(id_of(&created).starts_with("atlas-"), "{}", text(&created));
    assert!(!tracker.path().join("Software/parser/issues.org").exists());

    let quick = vissue_at(&repo, tracker.path(), &["q", "-p", "atlas", "named again"]);
    assert!(quick.status.success(), "{}", text(&quick));
    assert!(id_of(&quick).starts_with("atlas-"), "{}", text(&quick));
}

/// Standing inside `<root>/<prefix>/<name>/` is the current file's project,
/// ahead of the repository that contains the tracker.
#[test]
fn the_current_file_beats_the_repo() {
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

    let created = vissue_at(&inside, root, &["create", "--quiet", "where I stand"]);
    assert!(created.status.success(), "{}", text(&created));
    assert!(id_of(&created).starts_with("kept-"), "{}", text(&created));
    assert!(root.join("Software/kept/issues.org").is_file());
    assert!(!root.join("Software/parser/issues.org").exists());
    assert!(!root.join("Software/fromfile/issues.org").exists());
}

/// A `.project-ctx.toml` above the clone names Software. `create` and `q`
/// file in the repository: the remote name, or the top-level directory
/// when the repository has no remote.
#[test]
fn create_inside_a_repo_ignores_a_context_file_above_it() {
    let tracker = tracker();
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

    let created = vissue_at(
        &nested,
        tracker.path(),
        &["create", "--quiet", "from the clone"],
    );
    assert!(created.status.success(), "{}", text(&created));
    let id = id_of(&created);
    assert!(id.starts_with("parser-"), "{id}");
    assert!(tracker.path().join("Software/parser/issues.org").is_file());
    assert!(!tracker.path().join("Software/Software/issues.org").exists());

    let quick = vissue_at(&nested, tracker.path(), &["q", "quick from the clone"]);
    assert!(quick.status.success(), "{}", text(&quick));
    assert!(id_of(&quick).starts_with("parser-"), "{}", text(&quick));

    let folder = workspace.path().join("plain-folder");
    fs::create_dir_all(&folder).unwrap();
    git(&folder, &["init", "-q"]);
    let plain = vissue_at(&folder, tracker.path(), &["q", "from the folder"]);
    assert!(plain.status.success(), "{}", text(&plain));
    assert!(
        id_of(&plain).starts_with("plain-folder-"),
        "{}",
        text(&plain)
    );
    assert!(!tracker.path().join("Software/Software/issues.org").exists());
}

/// The remote's name matches an existing project directory by case.
#[test]
fn a_repo_name_matches_an_existing_project_by_case() {
    let tracker = tracker();
    let workspace = tempfile::tempdir().unwrap();
    fs::write(
        workspace.path().join(".project-ctx.toml"),
        "[project]\nname = \"Software\"\n",
    )
    .unwrap();
    let repo = workspace.path().join("checkout");
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

    let seeded = vissue_at(
        &repo,
        tracker.path(),
        &["create", "-p", "Parser", "--quiet", "seed"],
    );
    assert!(seeded.status.success(), "{}", text(&seeded));
    assert!(id_of(&seeded).starts_with("Parser-"), "{}", text(&seeded));

    let created = vissue_at(&repo, tracker.path(), &["q", "folded case"]);
    assert!(created.status.success(), "{}", text(&created));
    assert!(id_of(&created).starts_with("Parser-"), "{}", text(&created));
    assert!(!tracker.path().join("Software/parser/issues.org").exists());
    assert!(!tracker.path().join("Software/Software/issues.org").exists());
}

/// Outside a repository, a parent context file still names the project.
#[test]
fn a_context_file_names_the_project_outside_a_repo() {
    let tracker = tracker();
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
    assert!(!tracker.path().join("Software/Software/issues.org").exists());
}

/// A context file is also the current file's project, ahead of the repository.
#[test]
fn a_context_file_beats_the_repo() {
    let tracker = tracker();
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

    let created = vissue_at(&nested, tracker.path(), &["q", "from the context file"]);
    assert!(created.status.success(), "{}", text(&created));
    assert!(
        id_of(&created).starts_with("fromfile-"),
        "{}",
        text(&created)
    );
    assert!(!tracker.path().join("Software/parser/issues.org").exists());
}

/// No file and no repository leaves the prefix, which defaults to Software.
#[test]
fn the_prefix_is_the_last_resort() {
    let tracker = tracker();
    let work = tempfile::tempdir().unwrap();
    let plain = work.path().join("plain");
    // An empty `.git` is not a repository, and discovery stops there, so a
    // parent checkout cannot supply a name.
    fs::create_dir_all(plain.join(".git")).unwrap();

    let created = vissue_at(&plain, tracker.path(), &["create", "--quiet", "nowhere"]);
    assert!(created.status.success(), "{}", text(&created));
    assert!(
        id_of(&created).starts_with("Software-"),
        "{}",
        text(&created)
    );
    assert!(
        tracker
            .path()
            .join("Software/Software/issues.org")
            .is_file()
    );
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
}

#[test]
fn help_states_the_fallback_order() {
    for cmd in ["create", "q"] {
        let out = Command::new(env!("CARGO_BIN_EXE_vissue"))
            .args([cmd, "--help"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{cmd}");
        let help = String::from_utf8(out.stdout).unwrap();
        assert!(help.contains("current file"), "{cmd}:\n{help}");
        assert!(help.contains("remote name"), "{cmd}:\n{help}");
        assert!(help.contains("top-level directory"), "{cmd}:\n{help}");
        assert!(help.contains("prefix default"), "{cmd}:\n{help}");
    }
    let fold = Command::new(env!("CARGO_BIN_EXE_vissue"))
        .args(["fold", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8(fold.stdout).unwrap();
    assert!(help.contains(".project-ctx.toml"), "{help}");
    assert!(
        !help.contains("top-level directory"),
        "fold help grew the create fallback:\n{help}"
    );
}
