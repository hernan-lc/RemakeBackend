//! End-to-end checks of the `remake-dev` binary against throwaway workspaces.

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn remake(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_remake-dev"))
        .args(args)
        .env("REMAKE_DEV_ROOT", root)
        .env("DATABASE_URL", "postgres://cloud:pw@127.0.0.1:59999/cloud")
        .env_remove("CARGO_TARGET_DIR")
        .current_dir(root)
        .output()
        .unwrap()
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// A workspace with one crate whose tests are `tests_rs`.
fn workspace(tests_rs: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/demo\"]\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/demo/src")).unwrap();
    fs::write(
        root.join("crates/demo/Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(root.join("crates/demo/src/lib.rs"), tests_rs).unwrap();
    dir
}

#[test]
fn a_failing_test_gives_a_non_zero_exit_and_names_the_failure() {
    let dir = workspace(
        "#[cfg(test)] mod t { #[test] fn fine() {} #[test] fn broken() { assert_eq!(1, 2); } }",
    );
    let out = remake(dir.path(), &["--json", "test"]);
    assert_eq!(out.status.code(), Some(1));
    let v = json(&out);
    let run = &v["runs"][0];
    assert_eq!(run["kind"], "unit");
    assert_eq!(run["status"], "failed");
    assert_eq!(run["passed"], 1);
    assert_eq!(run["failed"], 1);
    assert_eq!(run["failures"][0]["name"], "t::broken");
}

#[test]
fn passing_tests_exit_zero_with_real_counts() {
    let dir = workspace("#[cfg(test)] mod t { #[test] fn a() {} #[test] fn b() {} }");
    let out = remake(dir.path(), &["--json", "test", "--unit"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = &json(&out)["runs"][0];
    assert_eq!(
        (run["status"].as_str(), run["passed"].as_u64()),
        (Some("passed"), Some(2))
    );
}

#[test]
fn a_run_with_no_tests_is_skipped_never_passed() {
    let dir = workspace("pub fn nothing() {}");
    let out = remake(dir.path(), &["--json", "test"]);
    assert_eq!(json(&out)["runs"][0]["status"], "skipped");
}

#[test]
fn report_is_versioned_json_and_does_not_invent_results() {
    let dir = workspace("#[cfg(test)] mod t { #[test] fn a() {} }");
    // Before any run: not_run, nothing installed.
    let fresh = json(&remake(dir.path(), &["report", "--format", "json"]));
    assert_eq!(fresh["schema_version"], 1);
    assert_eq!(fresh["project"], "RemakeBackend");
    assert_eq!(fresh["tests"]["status"], "not_run");
    assert_eq!(fresh["tests"]["passed"], 0);
    assert_eq!(fresh["build"]["status"], "not_run");
    assert_eq!(fresh["database"]["installed"], false);
    // After a real run the report carries it.
    remake(dir.path(), &["--json", "test"]);
    let after = json(&remake(dir.path(), &["report"]));
    assert_eq!(after["tests"]["status"], "passed");
    assert_eq!(after["tests"]["passed"], 1);
    assert!(after["platform"]["os"].is_string() && after["rust"]["rustc"].is_string());
    assert!(dir.path().join(".devtools/reports/latest.json").is_file());
}

#[test]
fn db_up_without_an_install_explains_how_to_install_and_fails() {
    let dir = workspace("pub fn x() {}");
    let out = remake(dir.path(), &["db", "up"]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("db install"), "{err}");
}

#[test]
fn doctor_fails_when_pgrust_is_missing_and_says_so_in_json() {
    let dir = workspace("pub fn x() {}");
    let out = remake(dir.path(), &["--json", "doctor"]);
    assert_eq!(out.status.code(), Some(1));
    let v = json(&out);
    assert_eq!(v["ok"], false);
    let binary = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "pgrust-binary")
        .unwrap();
    assert_eq!(binary["level"], "fail");
}

#[test]
fn setup_creates_the_config_and_the_gitignore_entry_once() {
    let dir = workspace("pub fn x() {}");
    fs::write(dir.path().join(".gitignore"), "/target").unwrap();
    assert_eq!(remake(dir.path(), &["setup"]).status.code(), Some(0));
    assert_eq!(remake(dir.path(), &["setup"]).status.code(), Some(0));
    let config = fs::read_to_string(dir.path().join("devtools.toml")).unwrap();
    assert!(config.contains("port = 55432") && config.contains("auto_build = false"));
    assert!(
        !config.contains("password"),
        "credentials never go in the versioned file"
    );
    let ignore = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert_eq!(ignore.matches(".devtools/").count(), 1, "{ignore}");
    assert!(ignore.starts_with("/target\n"));
}

#[test]
fn clean_removes_generated_files_and_keeps_the_install_and_data() {
    let dir = workspace("#[cfg(test)] mod t { #[test] fn a() {} }");
    remake(dir.path(), &["--json", "test"]);
    remake(dir.path(), &["report"]);
    let root = dir.path();
    fs::create_dir_all(root.join(".devtools/bin/pgrust")).unwrap();
    fs::write(root.join(".devtools/bin/pgrust/postgres"), b"bin").unwrap();
    fs::create_dir_all(root.join(".devtools/database/dev")).unwrap();
    fs::write(root.join(".devtools/database/dev/PG_VERSION"), b"18").unwrap();
    assert_eq!(remake(root, &["clean"]).status.code(), Some(0));
    assert!(!root.join(".devtools/reports/latest.json").exists());
    assert!(!root.join(".devtools/state/last_tests.json").exists());
    assert!(root.join(".devtools/bin/pgrust/postgres").is_file());
    assert!(root.join(".devtools/database/dev/PG_VERSION").is_file());
}
