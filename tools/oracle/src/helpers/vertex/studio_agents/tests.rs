#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/studio-agents")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_studio_agents() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("studio_agents"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert_eq!(seeds.len(), 3);
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "studio_agents"
    }));
}

#[test]
fn oracle_create_body_keeps_only_the_allowlisted_values() {
    let result = run_case("create-allowlist.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["headers"],
        json!({"X-Caller": "kept", "Content-Type": "application/json", "x-goog-api-key": "explicit"})
    );
    assert_eq!(
        calls[1]["body"],
        json!({
            "name": "my-agent",
            "base_agent": "waverunner",
            "instructions": "be nice",
            "base_environment": {"type": "remote"},
        })
    );
}

#[test]
fn oracle_empty_native_values_survive_the_allowlist() {
    let result = run_case("empty-native-values.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["body"],
        json!({"name": "empty", "base_agent": "", "instructions": "", "base_environment": {}})
    );
    assert_eq!(calls[1]["body"], json!({"name": "absent"}));
}

#[test]
fn oracle_identity_defaults_are_compared() {
    let result = run_case("absent-versus-null-identity.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["agent"],
        json!({"id": "request-name", "name": "request-name", "base_agent": "waverunner"})
    );
    assert_eq!(
        calls[1]["agent"],
        json!({"id": null, "name": null, "base_agent": "waverunner"})
    );
    assert_eq!(
        calls[2]["agent"],
        json!({"id": "provider-id", "name": "provider-id", "extra": {"k": [1_i32]}})
    );
}

#[test]
fn oracle_malformed_replies_fall_back_with_the_create_warning() {
    let created = run_case("malformed-create-warning.yaml");
    assert_eq!(created["calls"][0]["agent"]["id"], "request-name");
    assert_eq!(
        created["calls"][0]["warnings"],
        json!(["malformed_create_reply"])
    );
    let fetched = run_case("malformed-get-fallback.yaml");
    assert_eq!(fetched["calls"][0]["agent"]["name"], "request-name");
    assert_eq!(fetched["calls"][0]["warnings"], json!([]));
}

#[test]
fn oracle_malformed_pages_are_empty() {
    let result = run_case("malformed-list-empty.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(calls[0], json!({"agents": [], "next_page_token": null}));
    assert_eq!(
        calls[1],
        json!({"agent_versions": [], "next_page_token": null})
    );
}

#[test]
fn oracle_pagination_drops_empty_limits() {
    let result = run_case("zero-pagination-omitted.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0],
        json!({"url": "https://studio.example//v1beta/agents", "query": []})
    );
    assert_eq!(
        calls[1],
        json!({
            "url": "https://generativelanguage.googleapis.com/v1beta/agents/my-agent/versions",
            "query": [],
        })
    );
    assert_eq!(
        calls[2]["query"],
        json!([["pageSize", "7"], ["pageToken", "next token"]])
    );
}

#[test]
fn oracle_versions_and_deletes_project_native_fields() {
    let versions = run_case("versions-field.yaml");
    let calls = versions["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["url"],
        "https://custom.example/v1beta/agents/my-agent/versions"
    );
    assert_eq!(
        calls[0]["query"],
        json!([["pageSize", "3"], ["pageToken", "token"]])
    );
    assert_eq!(
        calls[1],
        json!({
            "agent_versions": [{"name": "agents/my-agent/versions/1", "state": "active"}],
            "next_page_token": "t",
        })
    );
    let deleted = run_case("successful-empty-delete.yaml");
    let delete_calls = deleted["calls"].as_array().unwrap();
    assert_eq!(
        delete_calls[0],
        json!({"name": "my-agent", "deleted": true})
    );
    assert_eq!(
        delete_calls[1],
        json!({"name": "my-agent", "deleted": true})
    );
}

#[test]
fn oracle_custom_base_refusal_keeps_the_environment_key_out() {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/studio-agents/custom-base-requires-explicit-key.yaml");
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let failure =
        super::Op::decode_and_run(&case.call.input).expect_err("the custom base is refused");
    assert!(failure.status.is_none());
    assert!(
        !failure.message.contains("env-google"),
        "{}",
        failure.message
    );
}
