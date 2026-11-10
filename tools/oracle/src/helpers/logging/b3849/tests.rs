#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde_json::{Value, json};

use super::Op;
use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::Helper as _;

fn run(input: Value) -> Value {
    let decoded: super::Input = serde_json::from_value(input).expect("the helper input decodes");
    Op::run(&decoded).expect("the helper runs")
}

fn proxy_owned_projection() -> Value {
    run(json!({
        "function": "for_destination",
        "span": {
            "name": "chat gpt-4",
            "attributes": {
                "gen_ai.operation.name": "chat",
                "http.route": "/v1/chat",
                "error.message": "boom",
                "http.url": "https://proxy/v1/chat?key=sk",
                "http.request.header.authorization": "Bearer x",
            },
            "events": [{"name": "exception", "attributes": {"exception.message": "boom", "exception.stacktrace": "trace"}}],
            "status": {"code": "error", "description": "failed"},
            "parent": "1112131415161718",
            "resource": {"service.name": "betterlite"},
        },
        "destination": {
            "endpoint": "http://collector:4318",
            "headers": {"x-api-key": "k"},
            "resource_attributes": {"tenant.id": "t-1", "service.name": "tenant"},
            "callback_name": "arize",
            "span_scope": "full",
        },
    }))
}

#[test]
fn for_destination_projects_the_tenant_view_and_resource() {
    let output = proxy_owned_projection();
    let result = &output["result"];
    assert_eq!(result["attributes"]["error.message"], "boom");
    assert_eq!(result["attributes"]["http.url"], "https://proxy/v1/chat");
    assert!(
        result["attributes"]
            .get("http.request.header.authorization")
            .is_none()
    );
    let event = &result["events"][0]["attributes"];
    assert!(event.get("exception.stacktrace").is_none());
    assert_eq!(result["status"]["description"], "failed");
    assert_eq!(result["parent"], "1112131415161718");
    assert_eq!(result["resource"]["service.name"], "tenant");
    assert_eq!(result["resource"]["tenant.id"], "t-1");
}

#[test]
fn scoped_roots_a_model_call_and_names_the_trace() {
    let output = run(json!({
        "function": "scoped",
        "span": {
            "name": "chat gpt-4",
            "attributes": {"gen_ai.operation.name": "chat"},
            "parent": "1112131415161718",
            "span_scope": "llm_only",
        },
        "span_scope": "llm_only",
    }));
    assert_eq!(output["result"]["parent"], Value::Null);
    assert_eq!(
        output["result"]["attributes"]["langfuse.trace.name"],
        "chat gpt-4"
    );
}

#[rstest]
#[case::base("http://collector:4318", "http://collector:4318/v1/traces")]
#[case::traces("http://collector:4318/v1/traces", "http://collector:4318/v1/traces")]
fn sink_key_records_the_account(#[case] endpoint: &str, #[case] normalized: &str) {
    let output = run(json!({
        "function": "sink_key",
        "endpoint": endpoint,
        "headers": {"Arize-Space-Id": "space"},
    }));
    assert_eq!(output["result"][0], normalized);
    assert_eq!(output["result"][1][0][0], "space_id");
}

#[rstest]
fn replays_the_otel_fanout_helper_corpus(
    #[files("fuzz/corpus/helper_case/otel-fanout/*.yaml")] path: PathBuf,
) {
    let case: HelperCase =
        serde_saphyr::from_str(&fs::read_to_string(&path).expect("a corpus case"))
            .expect("the corpus case parses");
    validate_helper(&path, &case).expect("the corpus case is valid");
    assert_eq!(case.call.operation, "b3849");
    assert!(Op::check(&case.call.input).is_ok(), "{}", path.display());
}
