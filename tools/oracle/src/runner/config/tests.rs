#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use super::prepare_gateway_configs;
use crate::OracleError;
use crate::case::{Case, decode_case};
use crate::runner::fixtures::FixtureGuard;
use crate::runner::group_key;

#[derive(Serialize)]
struct RequestDocument {
    method: &'static str,
    path: &'static str,
    headers: BTreeMap<String, String>,
    body: Value,
}

#[derive(Serialize)]
struct CaseDocument<'input> {
    name: &'static str,
    config: &'input str,
    env: BTreeMap<String, String>,
    request: RequestDocument,
    upstream: Vec<Value>,
    config_files: BTreeMap<String, String>,
}

fn case(config: &str, config_files: BTreeMap<String, String>) -> Case {
    let document = CaseDocument {
        name: "prepared",
        config,
        env: BTreeMap::new(),
        request: RequestDocument {
            method: "POST",
            path: "/v1/chat/completions",
            headers: BTreeMap::new(),
            body: Value::Null,
        },
        upstream: Vec::new(),
        config_files,
    };
    decode_case(
        Path::new("<prepared>"),
        &serde_json::to_vec(&document).unwrap(),
    )
    .unwrap()
}

fn included_case(config_files: BTreeMap<String, String>) -> Case {
    case(
        "general_settings:\n  master_key: oracle-master\nmodel_list:\n  - model_name: prepared\n    litellm_params: {model: openai/gpt-4o, api_base: \"{{upstream}}\"}\n",
        config_files,
    )
}

fn include_files() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "fragments/models.yaml".to_owned(),
            "model_list:\n  - model_name: isolated\n".to_owned(),
        ),
        (
            "nested/entry.yaml".to_owned(),
            "include:\n  - models.yaml\n".to_owned(),
        ),
    ])
}

fn no_fixtures() -> FixtureGuard {
    FixtureGuard { databases: None }
}

#[test]
fn isolates_proxy_trees() {
    let root = tempfile::tempdir().unwrap();
    let case = included_case(include_files());
    let (group_dir, first, second) =
        prepare_gateway_configs(root.path(), &case, &no_fixtures(), "http://127.0.0.1:9").unwrap();
    assert_eq!(first, group_dir.join("litellm").join("config.yaml"));
    assert_eq!(second, group_dir.join("betterlite").join("config.yaml"));
    assert_eq!(
        fs::read_to_string(&first).unwrap(),
        case.config.replace("{{upstream}}", "http://127.0.0.1:9")
    );
    let first_include = first.parent().unwrap().join("fragments/models.yaml");
    let second_include = second.parent().unwrap().join("fragments/models.yaml");
    let original = fs::read_to_string(&first_include).unwrap();
    assert_eq!(fs::read_to_string(&second_include).unwrap(), original);
    fs::write(&first_include, "changed\n").unwrap();
    assert_eq!(fs::read_to_string(&second_include).unwrap(), original);
}

#[test]
fn stops_on_write_failure() {
    let root = tempfile::tempdir().unwrap();
    let case = included_case(BTreeMap::from([(
        "fragments/models.yaml".to_owned(),
        "model_list: []\n".to_owned(),
    )]));
    let blocked = root.path().join("not-a-directory");
    fs::write(&blocked, "x").unwrap();
    let root_error = prepare_gateway_configs(&blocked, &case, &no_fixtures(), "http://127.0.0.1:9")
        .expect_err("the artifact root is a file");
    assert!(
        matches!(&root_error, OracleError::Filesystem { .. }),
        "{root_error:?}"
    );
    let directory = root.path().join("tree");
    fs::create_dir_all(directory.join("fragments/models.yaml")).unwrap();
    let include_error = super::write_include_tree(
        &directory,
        "general_settings: {}\n",
        None,
        &case.config_files,
        "http://127.0.0.1:9",
    )
    .expect_err("the include destination is a directory");
    let OracleError::Filesystem { path, .. } = include_error else {
        panic!("expected a filesystem error, got {include_error:?}");
    };
    assert_eq!(path, directory.join("fragments/models.yaml"));
}

#[test]
fn defaults_to_no_config_files() {
    let text = "name: legacy\nconfig: |\n  general_settings:\n    master_key: oracle-master\nenv: {}\nrequest: {method: GET, path: /health/liveliness, headers: {}, body: null}\nupstream: []\n";
    let case =
        decode_case(Path::new("<legacy>"), text.as_bytes()).expect("the legacy case decodes");
    assert!(case.config_files.is_empty());
    let root = tempfile::tempdir().unwrap();
    let (group_dir, first, second) =
        prepare_gateway_configs(root.path(), &case, &no_fixtures(), "http://127.0.0.1:9").unwrap();
    assert_eq!(first, group_dir.join("config.yaml"));
    assert_eq!(second, first);
}

#[test]
fn injects_the_database_url_per_root() {
    let case = included_case(BTreeMap::new());
    let first = super::database_document(&case.config, "postgresql://first").unwrap();
    let second = super::database_document(&case.config, "postgresql://second").unwrap();
    assert!(first.contains("postgresql://first"));
    assert!(second.contains("postgresql://second"));
    assert!(!first.contains("postgresql://second"));
}

#[test]
fn groups_by_include_contents() {
    let config = "general_settings:\n  master_key: oracle-master\n";
    let first = case(
        config,
        BTreeMap::from([("models.yaml".to_owned(), "model_list: []\n".to_owned())]),
    );
    let second = case(
        config,
        BTreeMap::from([(
            "models.yaml".to_owned(),
            "model_list:\n  - model_name: other\n".to_owned(),
        )]),
    );
    let repeated = case(
        config,
        BTreeMap::from([("models.yaml".to_owned(), "model_list: []\n".to_owned())]),
    );
    assert_eq!(group_key(&first), group_key(&repeated));
    assert_ne!(group_key(&first), group_key(&second));
}
