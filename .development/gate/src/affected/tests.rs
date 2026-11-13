#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use determinator::rules::DeterminatorRules;

use crate::affected::{RULES, Shard, affected, arguments, package_graph, packages, share};
use crate::error::GateError;
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
    let graph = package_graph(root).unwrap();
    let mut affected = packages(root, &base, &graph, false).unwrap();
    affected.sort_unstable();
    assert_eq!(affected, vec!["alpha".to_owned(), "beta".to_owned()]);
    assert_eq!(
        packages(root, &base, &graph, true).unwrap(),
        vec!["alpha".to_owned()]
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

#[test]
fn shards_take_the_largest_package_first_onto_the_lightest_shard() {
    let names: Vec<String> = ["e", "d", "c", "b", "a"].map(str::to_owned).to_vec();
    let seconds = BTreeMap::from([
        ("a".to_owned(), 10),
        ("b".to_owned(), 6),
        ("c".to_owned(), 5),
        ("d".to_owned(), 4),
        ("unused".to_owned(), 50),
    ]);
    assert_eq!(share(&names, &seconds, 1, 2), vec!["a", "d"]);
    assert_eq!(share(&names, &seconds, 2, 2), vec!["b", "c", "e"]);
    assert_eq!(share(&names, &seconds, 3, 3), vec!["c", "d"]);
}

#[test]
fn equal_shares_go_by_name_whatever_order_the_packages_come_in() {
    let seconds = BTreeMap::from([
        ("x".to_owned(), 3),
        ("y".to_owned(), 3),
        ("z".to_owned(), 0),
    ]);
    for names in [["x", "y", "z"], ["z", "y", "x"]] {
        let owned: Vec<String> = names.map(str::to_owned).to_vec();
        assert_eq!(share(&owned, &seconds, 1, 2), vec!["x", "z"]);
        assert_eq!(share(&owned, &seconds, 2, 2), vec!["y"]);
    }
}

#[test]
fn a_shard_is_one_of_n() {
    assert_eq!(
        share(&["only".to_owned()], &BTreeMap::new(), 6, 6),
        Vec::<&str>::new()
    );
    for (index, count) in [(0, 6), (7, 6), (1, 0)] {
        let shard = Shard {
            index: Some(index),
            count: Some(count),
        };
        assert!(
            matches!(
                affected(Path::new("no-such-checkout"), false, shard),
                Err(GateError::Shard { .. })
            ),
            "{index} of {count}"
        );
    }
}
