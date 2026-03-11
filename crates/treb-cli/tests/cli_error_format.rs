use assert_cmd::cargo::cargo_bin_cmd;

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

#[test]
fn unknown_command_uses_go_style_stderr() {
    let output = treb().arg("nonexistent").output().expect("failed to run treb nonexistent");

    assert!(!output.status.success(), "unknown command should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");

    assert_eq!(
        stderr,
        "Error: unknown command \"nonexistent\" for \"treb\"\nRun 'treb --help' for usage.\n"
    );
}

#[test]
fn unknown_flag_uses_go_style_stderr() {
    let output =
        treb().args(["list", "--nonexistent-flag"]).output().expect("failed to run treb list");

    assert!(!output.status.success(), "unknown flag should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");

    assert_eq!(
        stderr,
        "Error: unknown flag: --nonexistent-flag\nRun 'treb list --help' for usage.\n"
    );
}

#[test]
fn unknown_flag_json_uses_go_style_error_message() {
    let output = treb()
        .args(["list", "--json", "--nonexistent-flag"])
        .output()
        .expect("failed to run treb list --json");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "stdout should stay empty for json parse errors");

    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    let json: serde_json::Value =
        serde_json::from_str(&stderr).expect("stderr should be valid JSON");
    let error = json["error"].as_str().expect("json error should be a string");

    assert_eq!(error, "Error: unknown flag: --nonexistent-flag\nRun 'treb list --help' for usage.");
}
