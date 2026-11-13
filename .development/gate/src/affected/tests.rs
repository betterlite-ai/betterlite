#![cfg(test)]

use std::fs;
use std::path::Path;

use determinator::rules::DeterminatorRules;

use crate::affected::{RULES, arguments, packages};
use crate::git::git;

const LOCK: &str = "version = 4\n\n[[package]]\nname = \"alpha\"\nversion = \"0.0.0\"\n\n[[package]]\nname = \"beta\"\nversion = \"0.0.0\"\ndependencies = [\n \"alpha\",\n]\n\n[[package]]\nname = \"betterlite-dashboard-browser\"\nversion = \"0.0.0\"\n\n[[package]]\nname = \"gamma\"\nversion = \"0.0.0\"\n";

fn member(root: &Path, name: &str, dependency: &str) {
    fs::create_dir_all(root.join(name).join("src")).unwrap();
    fs::write(root.join(name).join("src/lib.rs"), "\n").unwrap();
    fs::write(
        root.join(name).join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\n{dependency}"),
    )
    .unwrap();
}

fn commit(root: &Path) -> String {
    git(root, &["add", "."]).unwrap();
    git(
        root,
        &[
            "-c",
            "user.name=test",
            "-c",
            "user.email=test.com",
            "commit",
            "-qm",
            "change",
        ],
    )
    .unwrap();
    git(root, &["rev-parse", "HEAD"]).unwrap().trim().to_owned()
}

#[test]
fn only_edited_leaves_out_the_packages_that_depend_on_the_edit() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    git(root, &["init", "--quiet", "-b", "main"]).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"alpha\", \"beta\", \"betterlite-dashboard-browser\", \"gamma\"]\nresolver = \"3\"\n",
    )
    .unwrap();
    fs::write(root.join("Cargo.lock"), LOCK).unwrap();
    member(root, "alpha", "");
    member(root, "beta", "alpha = { path = \"../alpha\" }\n");
    // rules.toml marks this package by name, and determinator fails on a name the workspace lacks
    member(root, "betterlite-dashboard-browser", "");
    member(root, "gamma", "");
    let base = commit(root);
    fs::write(root.join("alpha/src/lib.rs"), "pub const A: u8 = 1;\n").unwrap();
    commit(root);
    assert_eq!(
        packages(root, &base, false).unwrap(),
        vec!["-p alpha -p beta".to_owned()]
    );
    assert_eq!(
        packages(root, &base, true).unwrap(),
        vec!["-p alpha".to_owned()]
    );
}

#[test]
fn no_affected_package_prints_nothing() {
    assert!(arguments(3, Vec::new()).is_empty());
}

#[test]
fn every_member_affected_runs_the_workspace() {
    assert_eq!(
        arguments(2, vec!["betterlite-http", "betterlite"]),
        vec!["--workspace".to_owned()]
    );
}

#[test]
fn some_members_affected_print_sorted_package_flags() {
    assert_eq!(
        arguments(5, vec!["betterlite-oracle", "betterlite-a2a"]),
        vec!["-p betterlite-a2a -p betterlite-oracle".to_owned()]
    );
}

#[test]
fn the_rules_parse() {
    DeterminatorRules::parse(RULES).unwrap();
}
