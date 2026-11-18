use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;

use crate::semantic_guard::semantic_guard_route_loading::templates;

fn template_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/semantic_guard/semantic_guard_route_loading/route_templates")
}

fn template_names_on_disk() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(template_folder())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "yaml")
        })
        .map(|path| path.file_stem().unwrap().to_str().unwrap().to_owned())
        .collect();
    names.sort_unstable();
    names
}

#[test]
fn lists_the_builtin_templates_in_name_order() {
    assert_eq!(templates::list(), vec!["prompt_injection", "sql_injection"]);
}

#[test]
fn names_are_sorted() {
    let names = templates::list();
    assert!(names.is_sorted());
}

#[test]
fn lists_only_the_yaml_templates_on_disk() {
    assert_eq!(template_names_on_disk(), templates::list());
}

#[test]
fn no_listed_name_carries_the_extension() {
    assert!(templates::list().iter().all(|name| {
        !Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("yaml"))
    }));
}

#[rstest]
#[case::prompt_injection("prompt_injection")]
#[case::sql_injection("sql_injection")]
fn every_listed_template_loads(#[case] name: &str) {
    let definition = templates::load(name).unwrap();
    assert_eq!(definition.route_name, name);
}
