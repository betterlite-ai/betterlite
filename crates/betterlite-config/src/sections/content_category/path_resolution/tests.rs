#![cfg(test)]

use crate::loader::ConfigLoader;
use rstest::rstest;
use std::{collections::BTreeMap, env, fs, path::Path, slice};

use super::paths::{CategoryPathError, resolve_category_file_path};

#[test]
fn path_resolution_jails_and_resolves_relative_paths() {
    super::tests::path_resolution_uses_root_and_rejects_escape();
}

#[test]
fn path_resolution_never_rebases_absolute_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("category.yaml"), "keywords: []\n").unwrap();
    let outside = temp.path().join("category.yaml");
    let error =
        resolve_category_file_path(&outside, slice::from_ref(&root), &root, false).unwrap_err();
    assert!(matches!(error, CategoryPathError::OutsideRoots { .. }));
    let allowed =
        resolve_category_file_path(&outside, slice::from_ref(&root), &root, true).unwrap();
    assert_eq!(allowed, outside);
}

#[test]
fn existing_relative_path_is_inside_absolute_root() {
    let cwd = env::current_dir().unwrap();
    let temp = tempfile::tempdir_in(&cwd).unwrap();
    let file = temp.path().join("relative.yaml");
    fs::write(&file, "keywords: []").unwrap();
    let relative = file.strip_prefix(&cwd).unwrap();
    assert_eq!(
        resolve_category_file_path(relative, &[temp.path().to_path_buf()], temp.path(), false)
            .unwrap(),
        relative
    );
}

#[cfg(unix)]
#[rstest]
#[case::absolute("link/escape.yaml", true)]
#[case::missing("link/missing/file.yaml", true)]
#[case::suffix("link/escape.yaml", false)]
#[case::missing_suffix("link/missing/file.yaml", false)]
fn symlinks_cannot_escape_roots(#[case] name: &str, #[case] absolute: bool) {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("escape.yaml"), "keywords: []").unwrap();
    symlink(&outside, root.join("link")).unwrap();
    let file = if absolute {
        root.join(name)
    } else {
        Path::new(name).to_path_buf()
    };
    let error =
        resolve_category_file_path(&file, slice::from_ref(&root), &root, false).unwrap_err();
    assert!(matches!(error, CategoryPathError::OutsideRoots { .. }));
    let alias = temp.path().join("alias");
    symlink(&root, &alias).unwrap();
    super::paths::assert_within_data_roots(&root.join("missing.yaml"), &[alias]).unwrap();
}

#[rstest]
#[case::unset(None, false)]
#[case::empty(Some(""), false)]
#[case::false_value(Some("false"), false)]
#[case::true_value(Some("true"), true)]
#[case::mixed_case(Some("True"), true)]
fn environment_controls_external_files(#[case] value: Option<&str>, #[case] allowed: bool) {
    let lookup = value
        .map(|text| {
            (
                "LITELLM_CONTENT_FILTER_ALLOW_EXTERNAL_PATHS".to_owned(),
                text.to_owned(),
            )
        })
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let loaded = ConfigLoader { lookup }
        .load("router_settings: {num_retries: 3}")
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("external.yaml");
    fs::write(&file, "keywords: []").unwrap();
    let config = serde_saphyr::from_str(&format!(
        "category: external\naction: BLOCK\ncategory_file: {}\n",
        file.display()
    ))
    .unwrap();
    let result = super::loading::load_categories(
        &[config],
        &[temp.path().join("root")],
        temp.path(),
        &loaded.sections.content_category,
    );
    assert_eq!(result.loaded_categories.contains_key("external"), allowed);
}
