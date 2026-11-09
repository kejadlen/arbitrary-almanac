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
