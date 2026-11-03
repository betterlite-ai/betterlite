#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::{
    CATEGORIES_DIR, DATA_DIR, LEGACY_DATA_DIR, POLICY_TEMPLATES_DIR, data_roots_from_executable,
};
use crate::sections::content_filter_catalog::categories::available_content_categories;

#[test]
fn bundled_asset_paths_are_relative_to_the_installation_layout() {
    assert_eq!(
        Path::new(DATA_DIR).join("categories"),
        Path::new(CATEGORIES_DIR)
    );
    assert_eq!(
        Path::new(DATA_DIR).join("policy_templates"),
        Path::new(POLICY_TEMPLATES_DIR)
    );
    assert_eq!(
        Path::new(LEGACY_DATA_DIR),
        Path::new("assets/content_filter_legacy")
    );
}

#[test]
fn installed_category_discovery_works_without_the_build_checkout() {
    let installation = TempDir::new().unwrap();
    let executable_dir = installation.path().join("bin");
    let bundled_data = executable_dir.join(DATA_DIR);
    let categories_dir = bundled_data.join("categories");
    fs::create_dir_all(&categories_dir).unwrap();
    fs::write(
        categories_dir.join("installed.yaml"),
        "category_name: installed\ndisplay_name: Installed\n",
    )
    .unwrap();
    let executable = executable_dir.join("betterlite");
    let roots: [PathBuf; 2] = data_roots_from_executable(&executable).unwrap();
    assert!(!roots[0].starts_with(Path::new(env!("CARGO_MANIFEST_DIR"))));
    let root_refs: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
    let entries = available_content_categories(&root_refs);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "installed");
    assert_eq!(entries[0].display_name, "Installed");
}
