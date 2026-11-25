// ABOUTME: Verifies `codegraph --version` reports the workspace version.
// ABOUTME: The release workflow compares the git tag with this output.
use std::process::Command;

#[test]
fn version_prints_workspace_version() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "1.0.0");
    let out = Command::new(env!("CARGO_BIN_EXE_codegraph"))
        .arg("--version")
        .output()
        .expect("run codegraph --version");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!("codegraph {}", env!("CARGO_PKG_VERSION"))
    );
}
