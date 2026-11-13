#![cfg(test)]

use std::path::{Path, PathBuf};

use crate::shellcheck::{origin_of, scripts};

#[test]
fn checks_yaml_gives_every_command() {
    let catalog = Path::new(env!("CARGO_MANIFEST_DIR")).join("../checks.yaml");
    let found = scripts(&catalog).unwrap();
    let hooks = found
        .iter()
        .find(|script| script.origin.ends_with("checks.yaml command hooks"))
        .unwrap();
    assert!(hooks.run.contains("prek run"));
    assert!(
        found
            .iter()
            .any(|script| script.origin.ends_with("command mutants"))
    );
}

#[test]
fn finding_names_its_command() {
    let files = vec![
        (
            PathBuf::from("/tmp/x/0.sh"),
            "checks.yaml command fmt".to_owned(),
        ),
        (
            PathBuf::from("/tmp/x/1.sh"),
            "checks.yaml command hooks".to_owned(),
        ),
    ];
    assert_eq!(
        origin_of("/tmp/x/1.sh:2:5: note: Double quote [SC2086]", &files),
        "checks.yaml command hooks:1:5: note: Double quote [SC2086]"
    );
    assert_eq!(
        origin_of("shellcheck: bad option", &files),
        "shellcheck: bad option"
    );
}
