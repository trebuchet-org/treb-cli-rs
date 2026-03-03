//! Integration tests for `framework::workdir::TestWorkdir`.

mod framework;

use framework::workdir::TestWorkdir;
use std::fs;

fn minimal_fixture() -> std::path::PathBuf {
    TestWorkdir::fixture_dir("minimal-project")
}

#[test]
fn creates_workspace_with_copies() {
    let fixture = minimal_fixture();
    let w = TestWorkdir::new(&fixture);
    let root = w.path();

    // .treb/ directory exists.
    assert!(root.join(".treb").is_dir());

    // Copied items are regular files/dirs (not symlinks).
    assert!(root.join("foundry.toml").exists());
    assert!(!root.join("foundry.toml").is_symlink());

    assert!(root.join("src").is_dir());
    assert!(!root.join("src").is_symlink());
    assert!(root.join("src/Counter.sol").exists());

    assert!(root.join("script").is_dir());
    assert!(!root.join("script").is_symlink());
    assert!(root.join("script/Deploy.s.sol").exists());

    // Copies are independent — modifying the copy doesn't affect the fixture.
    let copy_content = fs::read_to_string(root.join("foundry.toml")).unwrap();
    let fixture_content = fs::read_to_string(fixture.join("foundry.toml")).unwrap();
    assert_eq!(copy_content, fixture_content);

    fs::write(root.join("foundry.toml"), "modified").unwrap();
    let fixture_after = fs::read_to_string(fixture.join("foundry.toml")).unwrap();
    assert_eq!(fixture_content, fixture_after, "fixture was mutated by copy");
}

#[test]
fn symlinks_are_symlinks_when_present() {
    // Create a fixture with a lib/ dir so we can test symlinking.
    let fixture_tmp = tempfile::tempdir().unwrap();
    let fixture = fixture_tmp.path();
    fs::create_dir_all(fixture.join("lib/dep")).unwrap();
    fs::write(fixture.join("lib/dep/file.txt"), "hello").unwrap();
    fs::write(fixture.join("foundry.toml"), "[profile.default]\n").unwrap();

    let w = TestWorkdir::new(fixture);
    let root = w.path();

    // lib/ should be a symlink.
    assert!(root.join("lib").is_symlink(), "lib/ should be a symlink");

    // Content is accessible through the symlink.
    let content = fs::read_to_string(root.join("lib/dep/file.txt")).unwrap();
    assert_eq!(content, "hello");
}

#[test]
fn missing_fixture_items_are_skipped() {
    // Fixture with only foundry.toml — no lib/, test/, etc.
    let fixture_tmp = tempfile::tempdir().unwrap();
    let fixture = fixture_tmp.path();
    fs::write(fixture.join("foundry.toml"), "[profile.default]\n").unwrap();

    let w = TestWorkdir::new(fixture);
    let root = w.path();

    assert!(root.join("foundry.toml").exists());
    assert!(!root.join("lib").exists());
    assert!(!root.join("test").exists());
    assert!(!root.join(".gitignore").exists());
    assert!(!root.join("remappings.txt").exists());
    assert!(!root.join("treb.toml").exists());
    assert!(!root.join("src").exists());
    assert!(!root.join("script").exists());
}

#[test]
fn treb_dir_accessor() {
    let fixture = minimal_fixture();
    let w = TestWorkdir::new(&fixture);
    assert!(w.treb_dir().ends_with(".treb"));
    assert!(w.treb_dir().is_dir());
}
