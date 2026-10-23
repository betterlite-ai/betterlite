#![cfg(test)]

use std::path::Path;

use super::dependency_names;

#[test]
fn every_dependency_table_counts_and_workspace_crates_do_not() {
    let manifest = "[package]\nname = \"betterlite-config\"\n\n[dependencies]\nserde = \"1\"\nbetterlite-http = { path = \"../http\" }\n\n[dev-dependencies]\ninsta = \"1\"\n\n[build-dependencies]\ncc = \"1\"\n\n[workspace.dependencies]\ntokio = \"1\"\n";
    let names = dependency_names(manifest, Path::new("Cargo.toml")).unwrap();
    assert_eq!(
        names.into_iter().collect::<Vec<_>>(),
        ["cc", "insta", "serde", "tokio"]
    );
}

#[test]
fn a_manifest_without_dependencies_has_none() {
    let names = dependency_names(
        "[package]\nname = \"betterlite\"\n",
        Path::new("Cargo.toml"),
    )
    .unwrap();
    assert!(names.is_empty());
}
