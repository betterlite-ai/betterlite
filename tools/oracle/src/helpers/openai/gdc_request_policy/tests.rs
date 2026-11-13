#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/gdc-request-policy")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_gdc_request_policy() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gdc_request_policy"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "gdc_request_policy"
    }));
}

#[test]
fn project_location_precedence_reaches_the_url() {
    let output = run_case("project-location-precedence.yaml");
    let urls = &output["scenarios"][0]["urls"];
    assert_eq!(
        urls[0]["value"],
        "https://gateway.example/v1/projects/call-project/locations/call-location/chat/completions"
    );
    assert_eq!(
        urls[1]["value"],
        "https://gateway.example/v1/projects/global-project/locations/global-location/chat/completions"
    );
    assert_eq!(
        urls[2]["value"],
        "https://gateway.example/v1/projects/alias-project/locations/alias-location/chat/completions"
    );
    let optional = &output["scenarios"][1]["urls"];
    assert_eq!(
        optional[0]["value"],
        "https://gateway.example/v1/projects/optional-project/locations/optional-location/chat/completions"
    );
    assert_eq!(
        optional[1]["value"],
        "https://gateway.example/v1/projects/optional-alias-project/locations/optional-alias-location/chat/completions"
    );
}

#[test]
fn a_full_project_url_is_returned_after_trimming() {
    let output = run_case("full-project-url-bypass.yaml");
    assert_eq!(
        output["scenarios"][0]["urls"][0]["value"],
        "https://gateway.example/v1/projects/prebuilt/locations/eu"
    );
    let urls = &output["scenarios"][1]["urls"];
    assert_eq!(
        urls[0]["value"],
        "https://gateway.example/v1/projects/prebuilt"
    );
    assert_eq!(urls[1]["value"], "https://gateway/v1/projects/prebuilt");
    assert_eq!(
        urls[2]["value"],
        "https://api.example/v1/projects/optional-project/locations/optional-location/chat/completions"
    );
}

#[test]
fn identifiers_are_checked_and_keep_one_terminal_newline() {
    let output = run_case("identifier-slash-and-terminal-newline.yaml");
    let urls = &output["scenarios"][0]["urls"];
    assert_eq!(urls[0], json!({"ok": false, "status": 401_i32}));
    assert_eq!(urls[1], json!({"ok": false, "status": 401_i32}));
    assert_eq!(
        urls[2]["value"],
        "https://gateway.example/v1/projects/project\n/locations/location/chat/completions"
    );
    assert_eq!(urls[3], json!({"ok": false, "status": 401_i32}));
    assert_eq!(urls[4], json!({"ok": false, "status": 401_i32}));
    assert_eq!(urls[5], json!({"ok": false, "status": 401_i32}));
}

#[test]
fn an_audience_keeps_only_the_scheme_and_netloc() {
    let output = run_case("audience-removes-path-and-query.yaml");
    assert_eq!(
        output["scenarios"][0]["audiences"],
        json!([
            {"ok": true, "value": "https://gateway.example"},
            {"ok": true, "value": "https://gateway.example"},
            {"ok": true, "value": "https://gateway.example"},
            {"ok": true, "value": "https://HTTP:"},
            {"ok": true, "value": "https://user:pw@host:8080"},
            {"ok": false, "status": 500_i32},
            {"ok": true, "value": "https://[fe80::1%25eth0]:443"},
            {"ok": true, "value": "https://u[]@v1.x"},
            {"ok": true, "value": "https://u[@v1.a]b"},
            {"ok": false, "status": 500_i32},
            {"ok": false, "status": 500_i32}
        ])
    );
}

#[test]
fn bool_settings_keep_their_original_string_form() {
    let output = run_case("bool-aliases-and-original-ca-path.yaml");
    let settings = &output["scenarios"][0]["settings"];
    assert_eq!(
        settings[0],
        json!({"ssl_verify": false, "gdc_token_caching": true, "caching_enabled": true})
    );
    assert_eq!(
        settings[1],
        json!({"ssl_verify": true, "gdc_token_caching": "ca.pem", "caching_enabled": true})
    );
    assert_eq!(
        settings[2],
        json!({"ssl_verify": false, "gdc_token_caching": true, "caching_enabled": true})
    );
    assert_eq!(
        settings[3],
        json!({"ssl_verify": "ca.pem", "gdc_token_caching": true, "caching_enabled": true})
    );
    assert_eq!(
        output["scenarios"][1]["settings"][0],
        json!({"ssl_verify": true, "gdc_token_caching": false, "caching_enabled": false})
    );
    assert_eq!(
        output["scenarios"][2]["settings"][0],
        json!({"ssl_verify": "ca.pem", "gdc_token_caching": false, "caching_enabled": false})
    );
}

#[test]
fn the_option_list_leads_with_the_vertex_names_and_the_body_drops_routing_fields() {
    let output = run_case("prefix-and-routing-field-removal.yaml");
    let supported = &output["scenarios"][0]["supported"];
    assert_eq!(supported[0][0], "vertex_project");
    assert_eq!(supported[0][1], "vertex_location");
    assert_eq!(supported[0].as_array().unwrap().len(), 33);
    assert!(!supported[0].as_array().unwrap().contains(&json!("user")));
    assert!(supported[1].as_array().unwrap().contains(&json!("user")));
    assert!(
        !supported[2]
            .as_array()
            .unwrap()
            .contains(&json!("response_format"))
    );
    assert!(supported[2].as_array().unwrap().contains(&json!("user")));
    let requests = &output["scenarios"][0]["requests"];
    assert_eq!(
        requests[0]["value"],
        json!({
            "model": "gemini-2.5-pro",
            "messages": [{"role": "user", "content": "hi"}],
            "temperature": 1_i32
        })
    );
    assert_eq!(requests[1]["value"]["model"], "gemini");
    assert_eq!(requests[2]["value"]["model"], "gdc/model");
    assert_eq!(
        requests[2]["value"],
        json!({"model": "gdc/model", "messages": [{"role": "user", "content": "hi"}]})
    );
    assert_eq!(
        requests[3]["value"],
        json!({
            "model": "model",
            "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]
        })
    );
}
