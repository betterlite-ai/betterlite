use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::support::{
    TOOL_CONVERSATION, content_of, conversation, field, guardrail, reply, sent, serve,
};
use crate::generic_guardrail_contracts::GuardrailInputType;

const NO_RECOVERY: &str = "      enable_retrieval: false\n";
const TWO_TOOLS: &str = r#"[{"role":"user","content":"find the answer"},{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"search","arguments":"{\"q\":\"x\"}"}},{"id":"c2","type":"function","function":{"name":"fetch","arguments":"{\"u\":\"y\"}"}}]},{"role":"tool","tool_call_id":"c1","content":"long tool output one"},{"role":"tool","tool_call_id":"c2","content":"long tool output two"}]"#;
const SHORT_TOOL: &str =
    r#"[{"role":"user","content":"find the answer"},{"role":"tool","content":"tiny"}]"#;
const BARE_TOOL: &str = r#"[{"role":"tool","content":"long tool output one"}]"#;

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

#[tokio::test]
async fn compresses_one_tool_output_apply_guardrail() {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, NO_RECOVERY);
    let mut data = BTreeMap::new();
    let output = guardrail
        .apply(
            conversation(TOOL_CONVERSATION, ""),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(content_of(&output, 2), text("short"));
    assert_eq!(content_of(&output, 0), text("find the answer"));
    let requests = sent(&server);
    assert_eq!(requests.len(), 1);
    let request = requests.first().unwrap();
    assert_eq!(request.path, "/api/compress/question-specific/");
    assert_eq!(request.api_key, "test-key");
    assert_eq!(
        field(&request.body, "context"),
        text("long tool output one")
    );
    assert_eq!(field(&request.body, "query"), text(r#"search: {"q":"x"}"#));
    assert_eq!(
        field(&request.body, "compression_model_name"),
        text("latte_v2")
    );
    assert_eq!(
        field(&request.body, "coarse"),
        GuardrailValue::Boolean(true)
    );
}

#[tokio::test]
async fn compresses_a_batch_by_position_apply_guardrail() {
    let server = serve(200, &reply(&[("first", 10, 4), ("second", 20, 8)])).await;
    let guardrail = guardrail(&server.base, NO_RECOVERY);
    let output = guardrail
        .apply(
            conversation(TWO_TOOLS, ""),
            &mut BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(content_of(&output, 2), text("first"));
    assert_eq!(content_of(&output, 3), text("second"));
    let requests = sent(&server);
    assert_eq!(
        requests.first().unwrap().path,
        "/api/compress/question-specific/batch"
    );
    let GuardrailValue::Array(batch) = field(&requests.first().unwrap().body, "inputs") else {
        panic!("a batch request carries its inputs as a list");
    };
    assert_eq!(batch.len(), 2);
    assert_eq!(
        field(batch.last().unwrap(), "query"),
        text(r#"fetch: {"u":"y"}"#)
    );
}

#[tokio::test]
async fn responses_are_left_alone_apply_guardrail() {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, NO_RECOVERY);
    let input = conversation(TOOL_CONVERSATION, "");
    let output = guardrail
        .apply(
            input.clone(),
            &mut BTreeMap::new(),
            GuardrailInputType::Response,
            None,
        )
        .await
        .unwrap();
    assert_eq!(output, input);
    assert!(sent(&server).is_empty());
}

#[rstest::rstest]
#[case::below_the_minimum(SHORT_TOOL)]
#[case::no_query_to_ask(BARE_TOOL)]
#[case::no_messages("[]")]
#[tokio::test]
async fn nothing_to_compress_apply_guardrail(#[case] rows: &str) {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, NO_RECOVERY);
    let input = conversation(rows, "");
    let output = guardrail
        .apply(
            input.clone(),
            &mut BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(output, input);
    assert!(sent(&server).is_empty());
}

#[tokio::test]
async fn request_without_structured_messages_apply_guardrail() {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, NO_RECOVERY);
    let input = conversation("null", r#","texts":["long tool output one"]"#);
    let output = guardrail
        .apply(
            input.clone(),
            &mut BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(output, input);
    assert!(sent(&server).is_empty());
}

#[rstest::rstest]
#[case::same_text("long tool output one")]
#[case::empty_text("")]
#[tokio::test]
async fn an_unchanged_result_is_not_a_compression_apply_guardrail(#[case] returned: &str) {
    let server = serve(200, &reply(&[(returned, 10, 10)])).await;
    let guardrail = guardrail(&server.base, NO_RECOVERY);
    let input = conversation(TOOL_CONVERSATION, "");
    let mut data = BTreeMap::new();
    let output = guardrail
        .apply(input.clone(), &mut data, GuardrailInputType::Request, None)
        .await
        .unwrap();
    assert_eq!(output, input);
    assert!(data.is_empty());
    assert_eq!(sent(&server).len(), 1);
}

#[tokio::test]
async fn records_the_statistics_apply_guardrail() {
    let server = serve(200, &reply(&[("first", 10, 4), ("second", 20, 8)])).await;
    let guardrail = guardrail(&server.base, NO_RECOVERY);
    let mut data = BTreeMap::new();
    guardrail
        .apply(
            conversation(TWO_TOOLS, ""),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    let GuardrailValue::Object(metadata) = data.get("metadata").unwrap() else {
        panic!("the record goes under metadata");
    };
    let GuardrailValue::Array(records) = metadata
        .get("standard_logging_guardrail_information")
        .unwrap()
    else {
        panic!("the records are a list");
    };
    let record = records.first().unwrap();
    assert_eq!(field(record, "guardrail_name"), text("squeeze"));
    assert_eq!(field(record, "guardrail_provider"), text("compresr"));
    assert_eq!(field(record, "guardrail_status"), text("success"));
    let stats = field(record, "guardrail_response");
    assert_eq!(
        field(&stats, "messages_compressed"),
        GuardrailValue::SignedInteger(2)
    );
    assert_eq!(
        field(&stats, "tokens_before"),
        GuardrailValue::SignedInteger(30)
    );
    assert_eq!(
        field(&stats, "tokens_after"),
        GuardrailValue::SignedInteger(12)
    );
    assert_eq!(
        field(&stats, "tokens_saved"),
        GuardrailValue::SignedInteger(18)
    );
    assert_eq!(field(&stats, "compression_model"), text("latte_v2"));
}

#[tokio::test]
async fn the_texts_channel_follows_the_compression_apply_guardrail() {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, NO_RECOVERY);
    let input = conversation(
        TOOL_CONVERSATION,
        r#","texts":["find the answer","long tool output one"]"#,
    );
    let output = guardrail
        .apply(
            input,
            &mut BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        output.texts,
        Some(vec!["find the answer".to_owned(), "short".to_owned()])
    );
}

fn bypass_data(value: &str) -> BTreeMap<String, GuardrailValue> {
    let body =
        format!(r#"{{"proxy_server_request":{{"headers":{{"x-compresr-bypass":"{value}"}}}}}}"#);
    serde_json::from_str(&body).unwrap()
}

#[tokio::test]
async fn the_header_skips_the_compression_bypass() {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(
        &server.base,
        "      allow_bypass_header: true\n      enable_retrieval: false\n",
    );
    let input = conversation(TOOL_CONVERSATION, "");
    let output = guardrail
        .apply(
            input.clone(),
            &mut bypass_data("TRUE"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(output, input);
    assert!(sent(&server).is_empty());
}

#[rstest::rstest]
#[case::not_allowed("      enable_retrieval: false\n", "true")]
#[case::another_value(
    "      allow_bypass_header: true\n      enable_retrieval: false\n",
    "false"
)]
#[tokio::test]
async fn the_header_does_not_apply_bypass(#[case] params: &str, #[case] value: &str) {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, params);
    let output = guardrail
        .apply(
            conversation(TOOL_CONVERSATION, ""),
            &mut bypass_data(value),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(content_of(&output, 2), text("short"));
    assert_eq!(sent(&server).len(), 1);
}
