#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;

use rstest::rstest;
use serde::Serialize;
use serde_json::Value;

use super::write_tree;
use crate::OracleError;
use crate::case::{Case, decode_case};

// 256 KiB of bodies fit four 64 KiB bodies exactly.
const BODIES_AT_TOTAL_LIMIT: usize = 4;

#[derive(Serialize)]
struct RequestDocument {
    method: &'static str,
    path: &'static str,
    headers: BTreeMap<String, String>,
    body: Value,
}

#[derive(Serialize)]
struct CaseDocument {
    name: &'static str,
    config: String,
    env: BTreeMap<String, String>,
    request: RequestDocument,
    upstream: Vec<Value>,
    config_files: BTreeMap<String, String>,
}

fn document(config_files: BTreeMap<String, String>) -> Vec<u8> {
    serde_json::to_vec(&CaseDocument {
        name: "include-fixture",
        config: "general_settings:\n  master_key: oracle-master\n".to_owned(),
        env: BTreeMap::new(),
        request: RequestDocument {
            method: "POST",
            path: "/v1/chat/completions",
            headers: BTreeMap::new(),
            body: Value::Null,
        },
        upstream: Vec::new(),
        config_files,
    })
    .unwrap()
}

fn decode(config_files: BTreeMap<String, String>) -> Result<Case, OracleError> {
    decode_case(Path::new("<fixture>"), &document(config_files))
}

fn rejected(config_files: BTreeMap<String, String>) -> String {
    let error = decode(config_files).expect_err("the fixture is rejected");
    let OracleError::CaseInvalid { message, .. } = error else {
        panic!("expected a case error, got {error:?}");
    };
    message
}

fn one_file(destination: &str, body: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(destination.to_owned(), body.to_owned())])
}

fn files_of(bodies: &[String]) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    for (index, body) in bodies.iter().enumerate() {
        files.insert(format!("fragments/{index}.yaml"), body.clone());
    }
    files
}

fn body_with_size(bytes: usize) -> String {
    let header = "model_list: []";
    let mut body = String::from(header);
    body.push_str(&" ".repeat(bytes.saturating_sub(header.len())));
    body
}

fn destination_with_size(bytes: usize) -> String {
    format!("{}.yaml", "a".repeat(bytes.saturating_sub(".yaml".len())))
}

fn repeated_bodies(count: usize) -> BTreeMap<String, String> {
    files_of(&vec!["model_list: []\n".to_owned(); count])
}

fn sized_body(size: usize) -> BTreeMap<String, String> {
    files_of(&[body_with_size(size)])
}

fn bodies_at_total_limit() -> BTreeMap<String, String> {
    files_of(&vec![body_with_size(64 * 1024); BODIES_AT_TOTAL_LIMIT])
}

fn bodies_past_total_limit() -> BTreeMap<String, String> {
    let mut bodies = vec![body_with_size(64 * 1024); BODIES_AT_TOTAL_LIMIT];
    bodies.push("a".to_owned());
    files_of(&bodies)
}

fn destination_of_size(size: usize) -> BTreeMap<String, String> {
    one_file(&destination_with_size(size), "model_list: []\n")
}

fn destination_with_components(count: usize) -> BTreeMap<String, String> {
    let names = (0..count)
        .map(|index| index.to_string())
        .collect::<Vec<_>>();
    one_file(&format!("{}.yaml", names.join("/")), "model_list: []\n")
}

#[rstest]
#[case::sixteen_files(repeated_bodies(16), true)]
#[case::seventeen_files(repeated_bodies(17), false)]
#[case::body_at_limit(sized_body(64 * 1024), true)]
#[case::body_past_limit(sized_body(64 * 1024 + 1), false)]
#[case::total_at_limit(bodies_at_total_limit(), true)]
#[case::total_past_limit(bodies_past_total_limit(), false)]
#[case::destination_at_limit(destination_of_size(512), true)]
#[case::destination_past_limit(destination_of_size(513), false)]
#[case::eight_components(destination_with_components(8), true)]
#[case::nine_components(destination_with_components(9), false)]
fn enforces_fixture_limits(#[case] files: BTreeMap<String, String>, #[case] accepted: bool) {
    let outcome = decode(files);
    if accepted {
        let case = outcome.expect("the fixture stays within the limits");
        assert!(!case.config_files.is_empty());
    } else {
        let error = outcome.expect_err("the fixture is past a limit");
        let OracleError::CaseInvalid { message, .. } = error else {
            panic!("expected a case error, got {error:?}");
        };
        assert!(message.contains("config_files"), "{message}");
    }
}

#[rstest]
#[case::absolute("/etc/models.yaml")]
#[case::drive_absolute("C:/models.yaml")]
#[case::drive_relative("C:models.yaml")]
#[case::backslash("fragments\\models.yaml")]
#[case::nul_byte("fragments/models\0.yaml")]
#[case::empty_component("fragments//models.yaml")]
#[case::trailing_separator("fragments/models.yaml/")]
#[case::dot("fragments/./models.yaml")]
#[case::dot_dot("fragments/../models.yaml")]
#[case::parent("../models.yaml")]
#[case::empty("")]
#[case::reserved_root("config.yaml")]
#[case::wrong_suffix("fragments/models.txt")]
#[case::uppercase_suffix("fragments/models.YAML")]
fn rejects_unsafe_destination(#[case] destination: &str) {
    let message = rejected(one_file(destination, "model_list: []\n"));
    assert!(message.contains("config_files["), "{message}");
}

#[rstest]
#[case::file_beside_directory("a.yaml", "a.yaml/b.yaml")]
#[case::directory_below_file("fragments/models.yaml", "fragments/models.yaml/extra.yaml")]
fn rejects_path_collision(#[case] ancestor: &str, #[case] nested: &str) {
    let mut files = one_file(ancestor, "model_list: []\n");
    files.insert(nested.to_owned(), "model_list: []\n".to_owned());
    let message = rejected(files);
    assert!(message.contains("collides"), "{message}");
}

#[rstest]
#[case::general_settings("general_settings:\n  litellm_license: fixture\n")]
#[case::environment_variables("environment_variables:\n  LITELLM_LICENSE: fixture\n")]
#[case::beside_a_model_list("model_list: []\ngeneral_settings:\n  litellm_license: fixture\n")]
fn rejects_included_license(#[case] body: &str) {
    let message = rejected(one_file("fragments/license.yaml", body));
    assert!(message.contains("LITELLM_LICENSE"), "{message}");
}

#[test]
fn preserves_yaml_and_mock_url() {
    let models = "model_list:\n  - model_name: preserved\n    litellm_params: {model: openai/gpt-4o, api_base: \"{{upstream}}/v1\"}\n";
    let case = decode(BTreeMap::from([
        ("fragments/models.yaml".to_owned(), models.to_owned()),
        ("fragments/empty.yaml".to_owned(), String::new()),
        (
            "fragments/list.yaml".to_owned(),
            "- one\n- two\n".to_owned(),
        ),
    ]))
    .expect("the fixture decodes");
    let directory = tempfile::tempdir().expect("a temporary directory");
    write_tree(directory.path(), &case.config_files, "http://127.0.0.1:9")
        .expect("the fixture writes");
    let read = |name: &str| {
        fs::read_to_string(directory.path().join(name)).expect("the fixture reads back")
    };
    assert_eq!(
        read("fragments/models.yaml"),
        models.replace("{{upstream}}", "http://127.0.0.1:9")
    );
    assert_eq!(read("fragments/empty.yaml"), "");
    assert_eq!(read("fragments/list.yaml"), "- one\n- two\n");
}

fn replay_valid_seed(path: &Path) -> usize {
    let bytes = fs::read(path).expect("a valid seed reads");
    let case = decode_case(path, &bytes).expect("a valid include seed decodes");
    assert!(!case.config_files.is_empty());
    1
}

fn replay_entry(path: &Path) -> usize {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if name == "config-includes" {
        let mut seeds = fs::read_dir(path)
            .expect("the include seed folder reads")
            .map(|entry| entry.expect("a seed entry").path())
            .collect::<Vec<_>>();
        seeds.sort();
        return seeds.iter().map(|seed| replay_valid_seed(seed)).sum();
    }
    if !name.starts_with("config-includes-") || !path.is_file() {
        return 0;
    }
    let bytes = fs::read(path).expect("a rejected seed reads");
    assert!(
        decode_case(path, &bytes).is_err(),
        "{} should be rejected",
        path.display()
    );
    1
}

#[test]
fn replays_the_http_case_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/http_case");
    let mut paths = fs::read_dir(root)
        .expect("the corpus folder reads")
        .map(|entry| entry.expect("a corpus entry").path())
        .collect::<Vec<_>>();
    paths.sort();
    let replayed: usize = paths.iter().map(|path| replay_entry(path)).sum();
    assert_eq!(replayed, 7);
}
