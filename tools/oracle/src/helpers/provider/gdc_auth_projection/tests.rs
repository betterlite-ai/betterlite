#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/gdc-auth-projection")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

fn cache_column(output: &Value, scenario: usize, validation: usize, column: usize) -> Vec<String> {
    output["scenarios"][scenario]["validations"][validation]["cache"]
        .as_array()
        .expect("a cache list")
        .iter()
        .map(|entry| {
            entry[column]
                .as_str()
                .expect("a cache key field")
                .to_owned()
        })
        .collect()
}

fn identities(output: &Value, scenario: usize, validation: usize) -> Vec<String> {
    cache_column(output, scenario, validation, 1)
}

fn audiences(output: &Value, scenario: usize, validation: usize) -> Vec<String> {
    cache_column(output, scenario, validation, 0)
}

fn project_of(identity: &str) -> String {
    let descriptor: Value = serde_json::from_str(identity).expect("a descriptor");
    descriptor["project"]
        .as_str()
        .expect("a project")
        .to_owned()
}

#[test]
fn helper_seeds_select_gdc_auth_projection() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gdc_auth_projection"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "gdc_auth_projection"
    }));
}

#[test]
fn invalid_json_is_a_bearer_token_and_a_json_literal_is_rejected() {
    let output = run_case("invalid-json-is-bearer.yaml");
    assert_eq!(
        output["scenarios"][0]["keys"],
        json!([
            {"ok": true, "kind": "bearer"},
            {"ok": true, "kind": "bearer"},
            {"ok": true, "kind": "bearer"},
            {"ok": true, "kind": "bearer"},
            {"ok": false, "status": 401_i32}
        ])
    );
    let headers = &output["scenarios"][0]["validations"][0]["headers"];
    assert_eq!(headers["Authorization"], "Bearer sk-plain-token");
    assert_eq!(headers["Content-Type"], "application/json");
    assert_eq!(headers["x-goog-user-project"], "projects/project-a");
    assert!(
        output["scenarios"][0]["validations"][0]["cache"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn header_casing_follows_the_source_dict_rules() {
    let output = run_case("invalid-json-is-bearer.yaml");
    let headers = &output["scenarios"][0]["validations"][1]["headers"];
    assert_eq!(headers["Authorization"], "Bearer own");
    assert_eq!(headers["authorization"], "lower");
    assert_eq!(headers["x-goog-user-project"], "projects/project-a");
    let base_headers = &output["scenarios"][0]["validations"][2]["headers"];
    assert_eq!(base_headers["Authorization"], "Bearer sk-plain-token");
    assert_eq!(base_headers["x-goog-user-project"], "projects/base-project");
}

#[test]
fn other_google_credential_kinds_are_authentication_failures() {
    let output = run_case("valid-json-other-credential-rejected.yaml");
    let keys = output["scenarios"][0]["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 8);
    assert!(
        keys.iter()
            .all(|key| key == &json!({"ok": false, "status": 401_i32}))
    );
    let validations = output["scenarios"][0]["validations"].as_array().unwrap();
    assert!(
        validations
            .iter()
            .all(|validation| validation == &json!({"ok": false, "status": 401_i32}))
    );
}

#[test]
fn the_cache_key_is_the_stripped_audience_and_the_exact_credential() {
    let output = run_case("gdch-audience-and-credential-identity.yaml");
    assert_eq!(
        output["scenarios"][0]["keys"],
        json!([{"ok": true, "kind": "service_account"}])
    );
    let first = identities(&output, 0, 0);
    assert_eq!(first.len(), 1);
    assert_eq!(audiences(&output, 0, 0), vec!["https://gateway.example"]);
    assert_eq!(project_of(&first[0]), "credential-project");
    let repeated = identities(&output, 0, 1);
    assert_eq!(repeated, first);
    let second_key = identities(&output, 0, 2);
    assert_eq!(second_key.len(), 2);
    assert_ne!(second_key[0], second_key[1]);
    assert_eq!(project_of(&second_key[1]), "other-project");
    let third_audience = audiences(&output, 0, 3);
    assert_eq!(
        third_audience,
        vec![
            "https://gateway.example",
            "https://gateway.example",
            "https://other.example:8443"
        ]
    );
    assert_eq!(identities(&output, 0, 3).len(), 3);
    let authorizations: Vec<&str> = (0..4)
        .map(|validation| {
            output["scenarios"][0]["validations"][validation]["headers"]["Authorization"]
                .as_str()
                .expect("an authorization")
        })
        .collect();
    assert_eq!(authorizations, vec!["Bearer fixture-token"; 4]);
}

#[test]
fn the_service_token_overwrites_exact_authorization_and_every_quota_variant() {
    let output = run_case("authorization-and-quota-header-casing.yaml");
    let first = &output["scenarios"][0]["validations"][0]["headers"];
    assert_eq!(first["Authorization"], "Bearer fixture-token");
    assert_eq!(first["authorization"], "lowercase");
    assert_eq!(first["Content-Type"], "text/plain");
    assert_eq!(first["x-goog-user-project"], "projects/project-a");
    let quota_count = first
        .as_object()
        .unwrap()
        .keys()
        .filter(|name| name.eq_ignore_ascii_case("x-goog-user-project"))
        .count();
    assert_eq!(quota_count, 1);

    let second = &output["scenarios"][0]["validations"][1]["headers"];
    assert_eq!(second["content-type"], "text/plain");
    assert_eq!(second["x-goog-user-project"], "projects/project-a");
    let second_quota = second
        .as_object()
        .unwrap()
        .keys()
        .filter(|name| name.eq_ignore_ascii_case("x-goog-user-project"))
        .count();
    assert_eq!(second_quota, 1);

    let third = &output["scenarios"][0]["validations"][2]["headers"];
    assert_eq!(third["Content-type"], "text/plain");
    assert_eq!(third["Content-Type"], "application/json");

    let fourth = &output["scenarios"][0]["validations"][3]["headers"];
    assert_eq!(fourth["Authorization"], "Bearer own");
}
