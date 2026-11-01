//! `vissue hud` through a fake rofi, so CI never needs a compositor.

#![allow(missing_docs)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_vissue")
}

fn copy_fixture(dest: &Path) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixture_vault");
    copy_dir(&src, dest);
}

fn copy_dir(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dest.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), to).unwrap();
        }
    }
}

/// Write an executable and hand back a path that was never open for writing.
///
/// A file written in one thread and exec'd moments later can fail with
/// ETXTBSY: another thread forking in between inherits the still-open
/// descriptor, and the kernel refuses to exec a file someone holds open for
/// writing. Renaming into place means the path being exec'd never was.
fn install_executable(path: &Path, body: &str) {
    let staging = path.with_extension("staging");
    fs::write(&staging, body).unwrap();
    let mut perm = fs::metadata(&staging).unwrap().permissions();
    perm.set_mode(0o755);
    fs::set_permissions(&staging, perm).unwrap();
    fs::rename(&staging, path).unwrap();
}

fn write_fake_rofi(dir: &Path, script: &str) -> PathBuf {
    let path = dir.join("fake-rofi");
    install_executable(&path, script);
    path
}

/// A fake rofi that answers each successive call from `replies`.
///
/// `run` calls the picker more than once for the actions that prompt for
/// text, so a fake that always says the same thing cannot reach the second
/// half of those paths.
fn write_scripted_rofi(dir: &Path, replies: &[(&str, i32)]) -> PathBuf {
    let mut script = String::from(
        "#!/bin/sh\n\
         count_file=\"$(dirname \"$0\")/calls\"\n\
         n=$(cat \"$count_file\" 2>/dev/null || echo 0)\n\
         n=$((n + 1))\n\
         echo \"$n\" > \"$count_file\"\n\
         cat > /dev/null\n",
    );
    for (i, (out, code)) in replies.iter().enumerate() {
        script.push_str(&format!(
            "if [ \"$n\" -eq {} ]; then printf '%s\\n' '{}'; exit {}; fi\n",
            i + 1,
            out,
            code
        ));
    }
    script.push_str("exit 1\n");
    write_fake_rofi(dir, &script)
}

fn hud(root: &Path, rofi: &Path, args: &[&str]) -> std::process::Output {
    let mut argv = vec!["--root", root.to_str().unwrap(), "hud", "--rofi"];
    argv.extend_from_slice(args);
    Command::new(bin())
        .args(argv)
        .env("VISSUE_ROFI", rofi)
        .env("VISSUE_AGENT", "rofi-picker")
        .output()
        .unwrap()
}

#[test]
fn hud_note_appends_to_the_logbook() {
    let tmp = tempfile::tempdir().unwrap();
    copy_fixture(tmp.path());
    // Alt+n on a row, then the note text at the second prompt.
    let fake = write_scripted_rofi(
        tmp.path(),
        &[
            ("atlas-2c3d  TODO  [#B]  Emit a summary table", 11),
            ("picked up from the picker", 0),
        ],
    );
    let out = hud(tmp.path(), &fake, &["--mode", "ready"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let shown = Command::new(bin())
        .args([
            "--root",
            tmp.path().to_str().unwrap(),
            "body-excerpt",
            "atlas-2c3d",
        ])
        .output()
        .unwrap();
    let body = String::from_utf8_lossy(&shown.stdout);
    let file = fs::read_to_string(tmp.path().join("Software/atlas/issues.org")).unwrap();
    assert!(
        file.contains("picked up from the picker"),
        "note missing from the file: {body}"
    );
}

#[test]
fn hud_note_with_empty_text_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    copy_fixture(tmp.path());
    let before = fs::read_to_string(tmp.path().join("Software/atlas/issues.org")).unwrap();
    // Alt+n, then an empty note: there is nothing to record.
    let fake = write_scripted_rofi(
        tmp.path(),
        &[
            ("atlas-2c3d  TODO  [#B]  Emit a summary table", 11),
            ("", 0),
        ],
    );
    let out = hud(tmp.path(), &fake, &["--mode", "ready"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let after = fs::read_to_string(tmp.path().join("Software/atlas/issues.org")).unwrap();
    assert_eq!(before, after, "an empty note edited the file");
}

#[test]
fn a_dismissed_picker_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    copy_fixture(tmp.path());
    let before = fs::read_to_string(tmp.path().join("Software/atlas/issues.org")).unwrap();
    // Escape leaves rofi with status 1, which maps to no action at all.
    let fake = write_scripted_rofi(tmp.path(), &[("", 1)]);
    let out = hud(tmp.path(), &fake, &["--mode", "list"]);
    assert!(
        out.status.success(),
        "dismissing is not a failure: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let after = fs::read_to_string(tmp.path().join("Software/atlas/issues.org")).unwrap();
    assert_eq!(before, after);
}

#[test]
fn hud_new_creates_an_issue_in_the_chosen_project() {
    let tmp = tempfile::tempdir().unwrap();
    copy_fixture(tmp.path());
    // First prompt picks the project, second takes the title.
    let fake = write_scripted_rofi(tmp.path(), &[("beacon", 0), ("Wire the picker", 0)]);
    let out = hud(tmp.path(), &fake, &["--mode", "new"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let file = fs::read_to_string(tmp.path().join("Software/beacon/issues.org")).unwrap();
    assert!(file.contains("Wire the picker"), "{file}");
}

#[test]
fn hud_new_without_a_title_creates_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    copy_fixture(tmp.path());
    let before = fs::read_to_string(tmp.path().join("Software/beacon/issues.org")).unwrap();
    let fake = write_scripted_rofi(tmp.path(), &[("beacon", 0), ("   ", 0)]);
    let out = hud(tmp.path(), &fake, &["--mode", "new"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let after = fs::read_to_string(tmp.path().join("Software/beacon/issues.org")).unwrap();
    assert_eq!(before, after);
}

#[test]
fn an_unknown_mode_names_the_modes_that_exist() {
    let tmp = tempfile::tempdir().unwrap();
    copy_fixture(tmp.path());
    let fake = write_scripted_rofi(tmp.path(), &[("", 0)]);
    let out = hud(tmp.path(), &fake, &["--mode", "sideways"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("sideways"), "{err}");
    assert!(err.contains("ready"), "{err}");
}

#[test]
fn hud_without_rofi_errors() {
    let out = Command::new(bin())
        .args(["--root", "/tmp/vissue-no-rofi", "hud", "--rofi"])
        .env_remove("VISSUE_ROFI")
        .env_remove("ROFI")
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("rofi is not installed"), "{err}");
}

#[test]
fn hud_claim_uses_alt_c_exit_code() {
    let tmp = tempfile::tempdir().unwrap();
    copy_fixture(tmp.path());
    let fake = write_fake_rofi(
        tmp.path(),
        r#"#!/bin/sh
# Claim an unclaimed ready row. atlas-1a2b is already held.
grep atlas-2c3d || head -n1
exit 10
"#,
    );
    let out = Command::new(bin())
        .args([
            "--root",
            tmp.path().to_str().unwrap(),
            "hud",
            "--rofi",
            "--mode",
            "ready",
        ])
        .env("VISSUE_ROFI", &fake)
        .env("VISSUE_AGENT", "rofi-picker")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let show = Command::new(bin())
        .args([
            "--root",
            tmp.path().to_str().unwrap(),
            "show",
            "atlas-2c3d",
            "--json",
        ])
        .output()
        .unwrap();
    // First ready row in the fixture is atlas-1a2b (already claimed) or
    // atlas-2c3d. The fake claims whichever row is first. Check someone
    // is now held by rofi-picker.
    let claims = Command::new(bin())
        .args(["--root", tmp.path().to_str().unwrap(), "claims", "--json"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&claims.stdout);
    assert!(
        text.contains("rofi-picker"),
        "claimed row missing: {text}\nshow={}",
        String::from_utf8_lossy(&show.stdout)
    );
}
