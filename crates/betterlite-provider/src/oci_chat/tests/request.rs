use betterlite_openai::chat::Role;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::oci_chat::payload::{OciChatRequest, transform_request};
use crate::oci_chat::tests::support::{
    FakePredicates, assert_json, body, build, chat_field, credentials, message, params, pick,
    rejected, to_json,
};

const LLAMA: &str = "meta.llama-3.3-70b-instruct";

const TOOLS: &str = r#"[{"type": "function", "function": {"name": "lookup", "description": "find it", "parameters": {"type": "object", "properties": {"q": {"type": "string"}}}}}]"#;

#[tokio::test]
async fn request_builds_a_generic_on_demand_payload() {
    let payload = build(LLAMA, "{}", false).await.unwrap();
    assert_json(
        &payload,
        r#"{
            "compartmentId": "compartment",
            "servingMode": {"servingType": "ON_DEMAND", "modelId": "meta.llama-3.3-70b-instruct"},
            "chatRequest": {
                "apiFormat": "GENERIC",
                "messages": [{"role": "USER", "content": [{"type": "TEXT", "text": "hi"}]}],
                "isStream": false,
                "maxTokens": 4096
            }
        }"#,
    );
}

#[rstest]
#[case::absent("{}", false)]
#[case::streaming(r#"{"stream": true}"#, true)]
#[case::not_streaming(r#"{"stream": false}"#, false)]
#[tokio::test]
async fn request_sets_is_stream_from_the_stream_param(
    #[case] optional: &str,
    #[case] streaming: bool,
) {
    assert_eq!(
        chat_field(LLAMA, optional, "isStream").await,
        Json::Bool(streaming)
    );
}

#[tokio::test]
async fn request_sets_is_stream_on_a_cohere_stream() {
    assert_eq!(
        chat_field("cohere.command-r", r#"{"stream": true}"#, "isStream").await,
        Json::Bool(true)
    );
    assert_eq!(
        chat_field("cohere.command-r", "{}", "isStream").await,
        Json::Null
    );
}

#[rstest]
#[case::dedicated_endpoint(
    r#"{"oci_serving_mode": "DEDICATED", "oci_endpoint_id": "ocid1.endpoint"}"#,
    r#"{"servingType": "DEDICATED", "endpointId": "ocid1.endpoint"}"#
)]
#[case::dedicated_falls_back_to_the_model(
    r#"{"oci_serving_mode": "DEDICATED"}"#,
    r#"{"servingType": "DEDICATED", "endpointId": "meta.llama-3.3-70b-instruct"}"#
)]
#[case::dedicated_null_endpoint(
    r#"{"oci_serving_mode": "DEDICATED", "oci_endpoint_id": null}"#,
    r#"{"servingType": "DEDICATED", "endpointId": "meta.llama-3.3-70b-instruct"}"#
)]
#[case::null_mode(
    r#"{"oci_serving_mode": null}"#,
    r#"{"servingType": "ON_DEMAND", "modelId": "meta.llama-3.3-70b-instruct"}"#
)]
#[case::on_demand_ignores_the_endpoint(
    r#"{"oci_serving_mode": "ON_DEMAND", "oci_endpoint_id": "ocid1.endpoint"}"#,
    r#"{"servingType": "ON_DEMAND", "modelId": "meta.llama-3.3-70b-instruct"}"#
)]
#[tokio::test]
async fn serving_mode_follows_the_oci_serving_mode_param(
    #[case] optional: &str,
    #[case] expected: &str,
) {
    let payload = build(LLAMA, optional, false).await.unwrap();
    assert_eq!(pick(&to_json(&payload), "servingMode"), body(expected));
}

#[rstest]
#[case::lower_case(
    r#"{"oci_serving_mode": "dedicated"}"#,
    "kwarg `oci_serving_mode` must be either 'ON_DEMAND' or 'DEDICATED'"
)]
#[case::not_text(
    r#"{"oci_serving_mode": 5}"#,
    "kwarg `oci_serving_mode` must be either 'ON_DEMAND' or 'DEDICATED'"
)]
#[case::endpoint_not_text(
    r#"{"oci_serving_mode": "DEDICATED", "oci_endpoint_id": 5}"#,
    "kwarg `oci_endpoint_id` must be a string"
)]
#[tokio::test]
async fn serving_mode_rejects_bad_values(#[case] optional: &str, #[case] message: &str) {
    let refusal = rejected(LLAMA, optional).await;
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.message, message);
}

#[tokio::test]
async fn request_builds_a_cohere_payload_with_preamble_and_history() {
    let payload = transform_request(
        "cohere.command-r",
        &[
            message(Role::System, "be"),
            message(Role::System, "nice"),
            message(Role::User, "first"),
            message(Role::Assistant, "answer"),
            message(Role::User, "second"),
        ],
        &params(r#"{"temperature": 0.2}"#),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    let OciChatRequest::Cohere(request) = &payload.chat_request else {
        panic!("a Cohere model builds a Cohere request");
    };
    assert_eq!(request.message, "second");
    assert_eq!(request.preamble_override.as_deref(), Some("be\nnice"));
    assert_eq!(request.chat_history.as_ref().unwrap().len(), 2);
    let sent = pick(&to_json(&payload), "chatRequest");
    assert_eq!(pick(&sent, "maxTokens"), body("4096"));
    assert_eq!(pick(&sent, "temperature"), body("0.2"));
    assert_eq!(pick(&sent, "apiFormat"), body(r#""COHERE""#));
}

#[tokio::test]
async fn request_leaves_the_cohere_preamble_out_without_a_system_message() {
    let payload = transform_request(
        "cohere.command-r",
        &[message(Role::User, "only")],
        &params("{}"),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    let OciChatRequest::Cohere(request) = &payload.chat_request else {
        panic!("a Cohere model builds a Cohere request");
    };
    assert_eq!(request.message, "only");
    assert!(request.preamble_override.is_none());
}

#[tokio::test]
async fn request_rejects_a_cohere_call_without_a_user_message() {
    let refusal = transform_request(
        "cohere.command-r",
        &[message(Role::System, "be")],
        &params("{}"),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap_err();
    assert_eq!(refusal.status, 400);
    assert_eq!(
        refusal.message,
        "No user message found \u{2014} Cohere models require at least one user message"
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(String::new()))]
#[tokio::test]
async fn request_requires_a_compartment_id(#[case] compartment: Option<String>) {
    let mut without = credentials();
    without.oci_compartment_id = compartment;
    let refusal = transform_request(
        LLAMA,
        &[message(Role::User, "hi")],
        &params("{}"),
        &without,
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap_err();
    assert_eq!(refusal.status, 400);
    assert_eq!(
        refusal.message,
        "oci_compartment_id is required for OCI chat requests. Pass it as optional_params or set the OCI_COMPARTMENT_ID env var."
    );
}

#[rstest]
#[case("openai.gpt-5", false, "maxCompletionTokens", "maxTokens")]
#[case("oci/OpenAI.gpt-5", false, "maxCompletionTokens", "maxTokens")]
#[case("openai.gpt-oss-120b", false, "maxTokens", "maxCompletionTokens")]
#[case("cohere.command-r", true, "maxTokens", "maxCompletionTokens")]
#[case("google.gemini-2.5-flash", true, "maxCompletionTokens", "maxTokens")]
#[case("google.gemini-2.5-flash", false, "maxTokens", "maxCompletionTokens")]
#[tokio::test]
async fn token_cap_follows_the_model(
    #[case] model: &str,
    #[case] reasoning: bool,
    #[case] key: &str,
    #[case] other_key: &str,
) {
    let payload = build(model, "{}", reasoning).await.unwrap();
    let sent = pick(&to_json(&payload), "chatRequest");
    assert_eq!(pick(&sent, key), body("4096"));
    assert_eq!(pick(&sent, other_key), Json::Null);
}

#[rstest]
#[case::max_tokens(LLAMA, r#"{"max_tokens": 7}"#, "maxTokens", "7")]
#[case::max_completion_tokens(LLAMA, r#"{"max_completion_tokens": 9}"#, "maxTokens", "9")]
#[case::first_key_wins(
    LLAMA,
    r#"{"max_completion_tokens": 9, "max_tokens": 7}"#,
    "maxTokens",
    "7"
)]
#[case::oci_alias(LLAMA, r#"{"maxTokens": 11}"#, "maxTokens", "11")]
#[case::openai_key_over_alias(LLAMA, r#"{"max_tokens": 7, "maxTokens": 11}"#, "maxTokens", "7")]
#[case::explicit_zero(LLAMA, r#"{"max_tokens": 0}"#, "maxTokens", "0")]
#[case::reasoning_model("openai.gpt-5", r#"{"max_tokens": 7}"#, "maxCompletionTokens", "7")]
#[tokio::test]
async fn token_cap_uses_the_first_token_key_present(
    #[case] model: &str,
    #[case] optional: &str,
    #[case] key: &str,
    #[case] expected: &str,
) {
    assert_eq!(chat_field(model, optional, key).await, body(expected));
    let other = if key == "maxTokens" {
        "maxCompletionTokens"
    } else {
        "maxTokens"
    };
    assert_eq!(chat_field(model, optional, other).await, Json::Null);
}

#[rstest]
#[case("low", "LOW")]
#[case("MINIMAL", "MINIMAL")]
#[case("disable", "NONE")]
#[case("Disable", "NONE")]
#[tokio::test]
async fn reasoning_effort_is_upper_cased_and_disable_becomes_none(
    #[case] given: &str,
    #[case] expected: &str,
) {
    let optional = format!(r#"{{"reasoning_effort": "{given}"}}"#);
    assert_eq!(
        chat_field(LLAMA, &optional, "reasoningEffort").await,
        Json::Text(expected.to_owned())
    );
}

#[tokio::test]
async fn reasoning_effort_that_is_not_text_passes_through() {
    assert_eq!(
        chat_field(LLAMA, r#"{"reasoning_effort": 5}"#, "reasoningEffort").await,
        body("5")
    );
}

#[tokio::test]
async fn tools_are_adapted_for_the_generic_vendor() {
    let optional = format!(r#"{{"tools": {TOOLS}}}"#);
    let Json::List(tools) = chat_field(LLAMA, &optional, "tools").await else {
        panic!("the generic request carries a list of tools");
    };
    assert_eq!(tools.len(), 1);
    assert_eq!(pick(&tools[0], "type"), body(r#""FUNCTION""#));
    assert_eq!(pick(&tools[0], "name"), body(r#""lookup""#));
}

#[tokio::test]
async fn tools_are_adapted_for_the_cohere_vendor() {
    let optional = format!(r#"{{"tools": {TOOLS}}}"#);
    let Json::List(tools) = chat_field("cohere.command-r", &optional, "tools").await else {
        panic!("the Cohere request carries a list of tools");
    };
    assert_eq!(tools.len(), 1);
    assert_eq!(pick(&tools[0], "name"), body(r#""lookup""#));
    let definitions = pick(&tools[0], "parameterDefinitions");
    assert_eq!(pick(&pick(&definitions, "q"), "type"), body(r#""str""#));
}

#[rstest]
#[case::text(r#"{"tools": "lookup"}"#)]
#[case::null(r#"{"tools": null}"#)]
#[tokio::test]
async fn tools_that_are_not_tool_definitions_are_rejected(#[case] optional: &str) {
    let refusal = rejected(LLAMA, optional).await;
    assert_eq!(refusal.status, 400);
    assert!(
        refusal
            .message
            .starts_with("`tools` is not a list of OpenAI tool definitions")
    );
}
