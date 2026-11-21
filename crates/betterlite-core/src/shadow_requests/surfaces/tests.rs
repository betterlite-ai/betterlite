#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource as _;
use rstest::rstest;

use crate::api::responses_chat_options::capabilities::CompletionBridgeProbe as _;
use crate::responses_bridge::BridgeRequest;
use crate::shadow_requests::Object;
use crate::shadow_requests::error::ShadowRequestError;

use super::environment::{NoBridge, NoParameters};
use super::{Surface, proxy_wire_body, surface_ops};

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

async fn chat_request(
    call_type: &str,
    kwargs: &Object,
    model_parameters: &Object,
) -> Result<Object, ShadowRequestError> {
    let ops = surface_ops(call_type).unwrap();
    (ops.chat_request)(kwargs, model_parameters).await
}

fn message_role(message: &Json) -> Option<&str> {
    let Json::Object(fields) = message else {
        return None;
    };
    match fields.get("role")? {
        Json::Text(role) => Some(role.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

#[rstest]
#[case::completion("completion", false, Surface::Chat)]
#[case::acompletion("acompletion", false, Surface::Chat)]
#[case::anthropic_messages("anthropic_messages", true, Surface::AnthropicMessages)]
#[case::aresponses("aresponses", true, Surface::Responses)]
#[case::responses("responses", true, Surface::Responses)]
fn surface_ops_returns_the_row_for_each_sampled_call_type(
    #[case] call_type: &str,
    #[case] wire_params: bool,
    #[case] surface: Surface,
) {
    let ops = surface_ops(call_type).unwrap();
    assert_eq!(ops.wire_params, wire_params);
    assert_eq!(ops.surface, surface);
}

#[rstest]
#[case::empty("")]
#[case::embedding("embedding")]
#[case::wrong_case("Completion")]
#[case::unknown_prefix("litellm_aresponses")]
fn unsampled_call_types_have_no_row(#[case] call_type: &str) {
    assert!(surface_ops(call_type).is_none());
}

#[tokio::test]
async fn chat_row_keeps_model_parameters_and_filters_messages() {
    let kwargs = object(r#"{"messages":[{"role":"user","content":"hi"},"text",7,null]}"#);
    let model_parameters = object(r#"{"model":"gpt-4o","temperature":0.5,"stop":[]}"#);
    let request = chat_request("completion", &kwargs, &model_parameters)
        .await
        .unwrap();
    assert_eq!(
        request,
        object(
            r#"{"model":"gpt-4o","temperature":0.5,"stop":[],"messages":[{"role":"user","content":"hi"}]}"#
        )
    );
}

#[rstest]
#[case::missing("{}")]
#[case::string(r#"{"messages":"hi"}"#)]
#[case::array_of_strings(r#"{"messages":["hi","there"]}"#)]
#[case::object(r#"{"messages":{"role":"user"}}"#)]
#[tokio::test]
async fn chat_row_gives_an_empty_message_list_for_a_non_array_value(#[case] source: &str) {
    let request = chat_request("acompletion", &object(source), &object("{}"))
        .await
        .unwrap();
    assert_eq!(request, object(r#"{"messages":[]}"#));
}

#[rstest]
#[case::body(
    r#"{"litellm_params":{"proxy_server_request":{"body":{"model":"m"}}}}"#,
    r#"{"model":"m"}"#
)]
#[case::empty_body(r#"{"litellm_params":{"proxy_server_request":{"body":{}}}}"#, "{}")]
#[case::no_body(r#"{"litellm_params":{"proxy_server_request":{}}}"#, "{}")]
#[case::no_request(r#"{"litellm_params":{}}"#, "{}")]
#[case::no_params("{}", "{}")]
#[case::params_not_an_object(r#"{"litellm_params":[]}"#, "{}")]
#[case::body_not_an_object(r#"{"litellm_params":{"proxy_server_request":{"body":"x"}}}"#, "{}")]
fn proxy_wire_body_reads_only_an_object_path_or_gives_an_empty_object(
    #[case] source: &str,
    #[case] expected: &str,
) {
    assert_eq!(proxy_wire_body(&object(source)), object(expected));
}

#[tokio::test]
async fn anthropic_row_takes_the_system_prompt_from_kwargs_first() {
    let kwargs = object(
        r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":"hi"}],"system":"be brief",
            "litellm_params":{"proxy_server_request":{"body":{"system":"inner"}}}}"#,
    );
    let model_parameters = object(r#"{"temperature":0.9}"#);
    let request = chat_request("anthropic_messages", &kwargs, &model_parameters)
        .await
        .unwrap();
    assert_eq!(
        request,
        object(
            r#"{"model":"claude-3-5-sonnet","messages":[{"role":"system","content":"be brief"},{"role":"user","content":"hi"}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_row_falls_back_to_the_wire_body_system() {
    let kwargs = object(
        r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":"hi"}],
            "litellm_params":{"proxy_server_request":{"body":{"system":"inner"}}}}"#,
    );
    let request = chat_request("anthropic_messages", &kwargs, &object("{}"))
        .await
        .unwrap();
    assert_eq!(
        request,
        object(
            r#"{"model":"claude-3-5-sonnet","messages":[{"role":"system","content":"inner"},{"role":"user","content":"hi"}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_row_reads_the_wire_body_when_the_kwargs_system_is_empty() {
    let kwargs = object(
        r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":"hi"}],"system":"",
            "litellm_params":{"proxy_server_request":{"body":{"system":"inner"}}}}"#,
    );
    let request = chat_request("anthropic_messages", &kwargs, &object("{}"))
        .await
        .unwrap();
    assert_eq!(
        request,
        object(
            r#"{"model":"claude-3-5-sonnet","messages":[{"role":"system","content":"inner"},{"role":"user","content":"hi"}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_row_copies_only_the_allowed_wire_body_keys() {
    let kwargs = object(
        r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":"hi"}],
            "litellm_params":{"proxy_server_request":{"body":{"max_tokens":64,"temperature":0,
                "top_p":0.9,"top_k":5,"reasoning_effort":"high","stop_sequences":["END"],
                "tools":[{"name":"lookup","description":"Find a record","input_schema":{"type":"object","properties":{"q":{"type":"string"}}}}],
                "tool_choice":{"type":"tool","name":"lookup"},"made_up_key":"x"}}}}"#,
    );
    let request = chat_request("anthropic_messages", &kwargs, &object(r#"{"top_p":0.1}"#))
        .await
        .unwrap();
    assert_eq!(
        request,
        object(
            r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":"hi"}],
                "max_tokens":64,"temperature":0,"top_p":0.9,"top_k":5,"reasoning_effort":"high","stop":["END"],
                "tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object","properties":{"q":{"type":"string"}}},"description":"Find a record"}}],
                "tool_choice":{"type":"function","function":{"name":"lookup"}}}"#
        )
    );
}

#[rstest]
#[case::missing(r#"{"model":"claude-3-5-sonnet"}"#)]
#[case::null(r#"{"model":"claude-3-5-sonnet","messages":null}"#)]
#[case::empty(r#"{"model":"claude-3-5-sonnet","messages":[]}"#)]
#[tokio::test]
async fn anthropic_row_treats_missing_messages_as_an_empty_conversation(#[case] source: &str) {
    let request = chat_request("anthropic_messages", &object(source), &object("{}"))
        .await
        .unwrap();
    assert_eq!(
        request,
        object(r#"{"model":"claude-3-5-sonnet","messages":[]}"#)
    );
}

#[rstest]
#[case::string(r#"{"model":"m","messages":"hi"}"#)]
#[case::number_item(r#"{"model":"m","messages":[1]}"#)]
#[case::object(r#"{"model":"m","messages":{"role":"user"}}"#)]
#[tokio::test]
async fn anthropic_row_rejects_messages_that_are_not_an_array_of_objects(#[case] source: &str) {
    let error = chat_request("anthropic_messages", &object(source), &object("{}"))
        .await
        .unwrap_err();
    assert!(matches!(error, ShadowRequestError::MessagesNotObjects));
}

#[rstest]
#[case::null(r#"{"model":null,"messages":null}"#)]
#[case::absent("{}")]
#[case::empty(r#"{"model":""}"#)]
#[tokio::test]
async fn anthropic_row_names_an_empty_model(#[case] source: &str) {
    let request = chat_request("anthropic_messages", &object(source), &object("{}"))
        .await
        .unwrap();
    assert_eq!(request, object(r#"{"model":"","messages":[]}"#));
}

#[rstest]
#[case::number(r#"{"model":7}"#)]
#[case::array(r#"{"model":["m"]}"#)]
#[tokio::test]
async fn anthropic_row_rejects_a_model_that_is_not_text(#[case] source: &str) {
    let error = chat_request("anthropic_messages", &object(source), &object("{}"))
        .await
        .unwrap_err();
    assert!(matches!(error, ShadowRequestError::ModelNotText));
}

#[tokio::test]
async fn responses_row_maps_the_wire_body_options_and_instructions() {
    let kwargs = object(
        r#"{"model":"gpt-4o","messages":"hello","instructions":"outer",
            "litellm_params":{"proxy_server_request":{"body":{"max_output_tokens":25,"made_up_key":"x","instructions":"inner"}}}}"#,
    );
    let request = chat_request("responses", &kwargs, &object(r#"{"temperature":0.9}"#))
        .await
        .unwrap();
    assert_eq!(
        request,
        object(
            r#"{"messages":[{"role":"system","content":"outer"},{"role":"user","content":"hello"}],"model":"gpt-4o","max_tokens":25}"#
        )
    );
}

#[tokio::test]
async fn responses_row_reads_the_wire_body_instructions_when_kwargs_has_none() {
    let kwargs = object(
        r#"{"model":"gpt-4o","messages":"hello",
            "litellm_params":{"proxy_server_request":{"body":{"instructions":"inner"}}}}"#,
    );
    let request = chat_request("aresponses", &kwargs, &object("{}"))
        .await
        .unwrap();
    assert_eq!(
        request,
        object(
            r#"{"messages":[{"role":"system","content":"inner"},{"role":"user","content":"hello"}],"model":"gpt-4o"}"#
        )
    );
}

#[tokio::test]
async fn responses_row_translates_the_optional_params_the_wire_body_carries() {
    let kwargs = object(
        r#"{"model":"gpt-4o","messages":"hello","litellm_params":{"proxy_server_request":{"body":{
            "tools":[{"type":"function","name":"lookup","description":"Find a record","parameters":{"type":"object","properties":{"q":{"type":"string"}}}}],
            "tool_choice":{"type":"function","name":"lookup"},"parallel_tool_calls":true,
            "text":{"format":{"type":"json_object"}},"reasoning":{"effort":"low"},
            "user":"u","safety_identifier":"s","context_management":{"type":"auto"}}}}}"#,
    );
    let request = chat_request("responses", &kwargs, &object("{}"))
        .await
        .unwrap();
    assert_eq!(
        request,
        object(
            r#"{"messages":[{"role":"user","content":"hello"}],"model":"gpt-4o",
                "tools":[{"type":"function","function":{"name":"lookup","description":"Find a record","parameters":{"type":"object","properties":{"q":{"type":"string"}}},"strict":false}}],
                "tool_choice":{"type":"function","function":{"name":"lookup"}},"parallel_tool_calls":true,
                "response_format":{"type":"json_object"},"reasoning_effort":"low",
                "user":"u","safety_identifier":"s","context_management":{"type":"auto"}}"#
        )
    );
}

#[tokio::test]
async fn responses_row_never_forwards_surface_only_keys() {
    let kwargs = object(
        r#"{"model":"gpt-4o","messages":"hello",
            "litellm_params":{"proxy_server_request":{"body":{"previous_response_id":"resp_1","store":false}}}}"#,
    );
    let request = chat_request("responses", &kwargs, &object("{}"))
        .await
        .unwrap();
    assert_eq!(
        request,
        object(r#"{"messages":[{"role":"user","content":"hello"}],"model":"gpt-4o"}"#)
    );
}

// NoCapabilities answers false, so a later system turn becomes a user message even for a capable model.
#[tokio::test]
async fn anthropic_row_converts_a_mid_conversation_system_turn_without_model_metadata() {
    let kwargs = object(
        r#"{"model":"claude-sonnet-5","messages":[{"role":"user","content":"hi"},
            {"role":"system","content":"be brief"},{"role":"user","content":"again"}]}"#,
    );
    let request = chat_request("anthropic_messages", &kwargs, &object("{}"))
        .await
        .unwrap();
    let Some(Json::List(messages)) = request.get("messages") else {
        panic!("messages is not a list");
    };
    let roles: Vec<Option<&str>> = messages.iter().map(message_role).collect();
    assert_eq!(roles, [Some("user"), Some("user"), Some("user")]);
}

// The bridge probe answers false, so a Responses-mode model drops the reasoning summary LiteLLM keeps.
#[tokio::test]
async fn responses_row_drops_a_reasoning_summary_for_a_responses_mode_model() {
    let kwargs = object(
        r#"{"model":"azure/codex-mini","messages":"hello",
            "litellm_params":{"proxy_server_request":{"body":{"reasoning":{"summary":"auto"}}}}}"#,
    );
    let request = chat_request("responses", &kwargs, &object("{}"))
        .await
        .unwrap();
    assert_eq!(
        request,
        object(r#"{"messages":[{"role":"user","content":"hello"}],"model":"azure/codex-mini"}"#)
    );
}

#[test]
fn the_shadow_environment_declares_no_parameters_and_no_completion_bridge() {
    let declared = NoParameters.supported_openai_params("m", None).unwrap();
    assert!(declared.is_none());
    let request = BridgeRequest {
        model: "m",
        custom_llm_provider: "openai",
        web_search_options: None,
        tools: &[],
        reasoning_effort: None,
        reasoning_summary: None,
        api_base: None,
        route_all_openai_to_responses: false,
    };
    assert!(!NoBridge.bridges_back(&request).unwrap());
}

#[tokio::test]
async fn responses_row_names_an_empty_model() {
    let request = chat_request(
        "responses",
        &object(r#"{"messages":"hello"}"#),
        &object("{}"),
    )
    .await
    .unwrap();
    assert_eq!(
        request,
        object(r#"{"messages":[{"role":"user","content":"hello"}],"model":""}"#)
    );
}
