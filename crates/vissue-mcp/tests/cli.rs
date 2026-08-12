use std::process::Command;

#[test]
fn version_flag_prints_the_binary_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_vissue-mcp"))
        .arg("--version")
        .output()
        .expect("run vissue-mcp --version");

    assert!(output.status.success(), "status: {}", output.status);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("vissue-mcp {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}
