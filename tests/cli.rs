#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

fn cmd() -> Command {
    cargo_bin_cmd!("arbitrary-almanac")
}

#[test]
fn shows_help() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: arbitrary-almanac"));
}

#[test]
fn prints_help_without_arguments() {
    cmd()
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: arbitrary-almanac"));
}

#[test]
fn shows_version() {
    cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("arbitrary-almanac "));
}

#[test]
fn generates_completions() {
    cmd()
        .args(["--completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("#compdef arbitrary-almanac"));
}

const RED: &str = r#"(local declare {:name :red :on :github.check_suite.completed
                :intent "Mark red builds" :may [:attest] :runs-per-hour 10})
(fn gather [{} event] {:repo event.repo :sha event.head_sha})
(fn policy [facts {: attest}]
  [(attest {:context "almanac/red" :state :failure :repo facts.repo :sha facts.sha})])
{: declare : gather : policy}"#;

const SUITE: &str = r#"{"repository": {"full_name": "o/r"},
  "check_suite": {"head_branch": "renovate/x", "head_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                  "conclusion": "failure", "id": 1, "pull_requests": [{"number": 3}]},
  "sender": {"login": "bot"}}"#;

/// A directory holding `workflow` as `red.fnl`, and a check suite payload.
fn workflows(workflow: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs_err::create_dir(dir.path().join("workflows")).unwrap();
    fs_err::write(dir.path().join("workflows/red.fnl"), workflow).unwrap();
    fs_err::write(dir.path().join("suite.json"), SUITE).unwrap();
    dir
}

#[test]
fn runs_almanacs_verbs_on_the_workflows_it_loads() {
    let dir = workflows(RED);
    cmd()
        .current_dir(dir.path())
        .arg("synth")
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""workflow": "red""#));
    cmd()
        .current_dir(dir.path())
        .env("ARBITRARY_ALMANAC_WORKFLOWS", dir.path().join("workflows"))
        .args([
            "simulate",
            "github.check_suite.completed",
            "suite.json",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Attest").and(predicate::str::contains("WouldApply")));
}

#[test]
fn refuses_to_run_when_a_workflow_does_not_load() {
    let dir = workflows("(oops");
    cmd()
        .current_dir(dir.path())
        .arg("synth")
        .assert()
        .failure()
        .stderr(predicate::str::contains("arbitrary_almanac::parse"));
}
