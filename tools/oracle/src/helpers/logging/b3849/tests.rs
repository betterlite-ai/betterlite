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

#[test]
fn for_destination_projects_the_tenant_attributes() {
    let output = run(json!({
        "function": "for_destination",
        "endpoint": "http://collector:4318",
        "span": {
            "name": "chat gpt-4",
            "attributes": {
                "gen_ai.operation.name": "chat",
                "http.route": "/v1/chat",
                "error.message": "boom",
                "http.url": "https://proxy/v1/chat?key=sk",
                "http.request.header.authorization": "Bearer x",
            },
        },
    }));
    let result = &output["result"];
    assert_eq!(result["attributes"]["error.message"], "boom");
    assert_eq!(result["attributes"]["http.url"], "https://proxy/v1/chat");
    assert!(
        result["attributes"]
            .get("http.request.header.authorization")
            .is_none()
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
    #[files("fuzz/corpus/helper_case/otel-fanout*")] path: PathBuf,
) {
    let case: HelperCase =
        serde_saphyr::from_str(&fs::read_to_string(&path).expect("a corpus case"))
            .expect("the corpus case parses");
    validate_helper(&path, &case).expect("the corpus case is valid");
    assert_eq!(case.call.operation, "b3849");
    assert!(Op::check(&case.call.input).is_ok(), "{}", path.display());
}
