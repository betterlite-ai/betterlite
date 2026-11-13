mod support;

use std::sync::Arc;

use betterlite_logging::rubrik::block_decision::ResponseBlockError;
use betterlite_logging::rubrik::tool_calls::ToolCallSource;
use betterlite_logging::rubrik_delivery::transport::{ClientTier, MODERATION_TIMEOUT};
use betterlite_logging::rubrik_moderation::apply::apply_guardrail;
use betterlite_logging::rubrik_moderation::context::{
    LoggingObject, LoggingObjectStash, RequestData, stash_block_context,
};
use betterlite_logging::rubrik_moderation::error::{ModerationError, ModifyResponseException};
use betterlite_logging::rubrik_moderation::guarded::guarded;
use betterlite_logging::rubrik_moderation::inputs::{GuardrailInputs, ModerationInputType};
use betterlite_logging::rubrik_moderation::prompt::moderate_prompt;
use betterlite_logging::rubrik_moderation::response::moderate_response;
use betterlite_logging::rubrik_moderation::service::service_response;
use betterlite_models::decisions::Json;
use rstest::rstest;
use tracing::Level;

use support::{
    BASE_URL, FarFutureClock, Harness, ScriptedReply, field, json, logging_object,
    transient_failure,
};

fn assert_same_inputs(returned: &GuardrailInputs, expected: &GuardrailInputs) {
    assert_eq!(returned.texts, expected.texts);
    assert_eq!(returned.tool_calls.len(), expected.tool_calls.len());
    assert_eq!(returned.structured_messages, expected.structured_messages);
    assert_eq!(returned.model, expected.model);
    assert_eq!(returned.tools, expected.tools);
}

fn text_inputs(texts: &[&str]) -> GuardrailInputs {
    GuardrailInputs {
        texts: texts.iter().map(|text| (*text).to_owned()).collect(),
        tool_calls: Vec::new(),
        structured_messages: Vec::new(),
        model: None,
        tools: None,
    }
}

fn text_field(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn request_fields(pairs: &[(&str, Json)]) -> RequestData {
    let mut data = RequestData::default();
    for (name, value) in pairs {
        data.fields.insert((*name).to_owned(), value.clone());
    }
    data
}

fn block(result: Result<GuardrailInputs, ModerationError>) -> ModifyResponseException {
    match result {
        Err(ModerationError::Blocked(exception)) => *exception,
        other => panic!("a block is expected, got {other:?}"),
    }
}

fn expected_response_route_payload() -> Json {
    json(
        r#"{
            "request": {"messages": null, "model": null, "tools": null},
            "response": {
                "id": "call-1",
                "object": "chat.completion",
                "created": 1700000000,
                "model": "",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "hello"},
                    "finish_reason": "stop"
                }]
            }
        }"#,
    )
}

fn expected_tool_call_payload() -> Json {
    json(
        r#"{
            "request": {
                "messages": [{"role": "user", "content": "weather?"}],
                "model": "gpt-4o",
                "tools": [{"type": "function", "function": {"name": "get_weather"}}]
            },
            "response": {
                "id": "call-1",
                "object": "chat.completion",
                "created": 1700000000,
                "model": "",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "The weather is sunny.",
                        "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{}"}}]
                    },
                    "finish_reason": "tool_calls"
                }]
            }
        }"#,
    )
}

fn expected_prompt_payload() -> Json {
    json(
        r#"{
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "part\n{\"x\":1}"}
            ],
            "tools": [],
            "user": "u1",
            "correlation_key": "ck"
        }"#,
    )
}

#[tokio::test]
async fn apply_guardrail_routes_request_inputs_to_prompt_moderation() {
    let harness = Harness::new(vec![ScriptedReply::json("{}")]);
    let inputs = GuardrailInputs {
        texts: Vec::new(),
        tool_calls: Vec::new(),
        structured_messages: vec![json(r#"{"role":"user","content":"hello"}"#)],
        model: Some("gpt-4o".to_owned()),
        tools: None,
    };
    let mut request_data = RequestData::default();
    let returned = apply_guardrail(
        &harness.logger,
        inputs.clone(),
        &mut request_data,
        ModerationInputType::Request,
        None,
    )
    .await
    .expect("prompt moderation passes the request through");
    assert_same_inputs(&returned, &inputs);
    let calls = harness.transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tier, ClientTier::Moderation);
    assert_eq!(
        calls[0].url,
        format!("{BASE_URL}/v1/before_prompt/openai/v1")
    );
    assert_eq!(calls[0].timeout, MODERATION_TIMEOUT);
    assert_eq!(
        calls[0].payload,
        json(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#)
    );
    assert!(harness.logs.with_level(Level::WARN).is_empty());
    assert!(harness.logs.with_level(Level::ERROR).is_empty());
}

#[tokio::test]
async fn apply_guardrail_routes_response_inputs_to_response_moderation() {
    let harness = Harness::new(vec![ScriptedReply::json(
        r#"{"choices":[{"message":{"content":"hello"}}]}"#,
    )]);
    let inputs = text_inputs(&["hello"]);
    let mut request_data = RequestData::default();
    request_data
        .fields
        .insert("litellm_call_id".to_owned(), text_field("call-1"));
    let returned = apply_guardrail(
        &harness.logger,
        inputs.clone(),
        &mut request_data,
        ModerationInputType::Response,
        None,
    )
    .await
    .expect("response moderation passes the response through");
    assert_same_inputs(&returned, &inputs);
    let calls = harness.transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].url,
        format!("{BASE_URL}/v1/after_completion/openai/v1")
    );
    assert_eq!(calls[0].payload, expected_response_route_payload());
}

#[tokio::test]
async fn apply_guardrail_returns_inputs_unchanged_for_an_unrecognized_type() {
    let harness = Harness::new(Vec::new());
    let inputs = text_inputs(&["hello"]);
    let mut request_data = RequestData::default();
    let returned = apply_guardrail(
        &harness.logger,
        inputs.clone(),
        &mut request_data,
        ModerationInputType::Unrecognized,
        None,
    )
    .await
    .expect("an unknown input type is not an error");
    assert_same_inputs(&returned, &inputs);
    assert!(harness.transport.calls().is_empty());
    assert!(harness.logs.events().is_empty());
}

#[tokio::test]
async fn guarded_rethrows_a_modify_response_exception_without_logging() {
    let harness = Harness::new(Vec::new());
    let exception = ModifyResponseException {
        message: "blocked".to_owned(),
        model: "gpt-4o".to_owned(),
        request_data: Box::new(RequestData::default()),
        guardrail_name: "safety".to_owned(),
    };
    let operation = async {
        Err::<GuardrailInputs, ModerationError>(ModerationError::Blocked(Box::new(exception)))
    };
    let error = guarded(operation, text_inputs(&["hello"]), "Response moderation")
        .await
        .expect_err("the block is rethrown");
    assert_eq!(error.message, "blocked");
    assert_eq!(error.model, "gpt-4o");
    assert_eq!(error.guardrail_name, "safety");
    assert!(harness.logs.events().is_empty());
}

#[tokio::test]
async fn guarded_logs_critical_and_returns_inputs_on_a_malformed_response() {
    let harness = Harness::new(Vec::new());
    let inputs = text_inputs(&["hello"]);
    let operation = async {
        Err::<GuardrailInputs, ModerationError>(ModerationError::Malformed(
            ResponseBlockError::MalformedResponse,
        ))
    };
    let returned = guarded(operation, inputs.clone(), "Response moderation")
        .await
        .expect("a malformed service response fails open");
    assert_same_inputs(&returned, &inputs);
    let errors = harness.logs.with_level(Level::ERROR);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].severity.as_deref(), Some("critical"));
    assert!(errors[0].message.contains("malformed"));
}

#[tokio::test]
async fn guarded_logs_error_and_returns_inputs_on_a_transient_failure() {
    let harness = Harness::new(Vec::new());
    let inputs = text_inputs(&["hello"]);
    let operation = async {
        Err::<GuardrailInputs, ModerationError>(ModerationError::Rubrik(transient_failure()))
    };
    let returned = guarded(operation, inputs.clone(), "Response moderation")
        .await
        .expect("a transient failure fails open");
    assert_same_inputs(&returned, &inputs);
    let errors = harness.logs.with_level(Level::ERROR);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].severity, None);
    assert!(
        errors[0]
            .message
            .contains("Response moderation hook failed")
    );
}

#[tokio::test]
async fn moderate_response_sends_normalized_tool_calls_and_the_request_envelope() {
    let harness = Harness::new(vec![ScriptedReply::json(
        r#"{"choices":[{"message":{"content":"The weather is sunny.","tool_calls":[{"id":"call_1"}]}}]}"#,
    )]);
    let logging_obj = logging_object(
        r#"{
            "messages": [{"role": "user", "content": "weather?"}],
            "model": "gpt-4o",
            "optional_params": {"tools": [{"type": "function", "function": {"name": "get_weather"}}]}
        }"#,
    );
    let inputs = GuardrailInputs {
        texts: vec!["The weather is sunny.".to_owned()],
        tool_calls: vec![ToolCallSource::Raw(json(
            r#"{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{}"}}"#,
        ))],
        structured_messages: Vec::new(),
        model: None,
        tools: None,
    };
    let mut request_data = RequestData::default();
    request_data
        .fields
        .insert("litellm_call_id".to_owned(), text_field("call-1"));
    let returned = moderate_response(
        &harness.logger,
        inputs.clone(),
        &mut request_data,
        Some(&logging_obj),
    )
    .await
    .expect("both tool calls came back unchanged");
    assert_same_inputs(&returned, &inputs);
    let calls = harness.transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].payload, expected_tool_call_payload());
}

#[tokio::test]
async fn moderate_response_returns_inputs_when_both_tool_calls_and_texts_are_empty() {
    let harness = Harness::new(Vec::new());
    let inputs = GuardrailInputs {
        texts: Vec::new(),
        tool_calls: Vec::new(),
        structured_messages: vec![json(r#"{"role":"user","content":"hello"}"#)],
        model: Some("gpt-4o".to_owned()),
        tools: None,
    };
    let mut request_data = RequestData::default();
    let returned = moderate_response(&harness.logger, inputs.clone(), &mut request_data, None)
        .await
        .expect("nothing to moderate");
    assert_same_inputs(&returned, &inputs);
    assert!(harness.transport.calls().is_empty());
    assert!(harness.logs.events().is_empty());
}

#[tokio::test]
async fn moderate_response_warns_when_the_logging_object_has_empty_call_details() {
    let harness = Harness::new(vec![ScriptedReply::json(
        r#"{"choices":[{"message":{"content":"hello"}}]}"#,
    )]);
    let logging_obj = logging_object("{}");
    let mut request_data = RequestData::default();
    let returned = moderate_response(
        &harness.logger,
        text_inputs(&["hello"]),
        &mut request_data,
        Some(&logging_obj),
    )
    .await
    .expect("the echoed response is not a block");
    assert_eq!(returned.texts, vec!["hello".to_owned()]);
    let warnings = harness.logs.with_level(Level::WARN);
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0].message,
        "Rubrik: logging_obj present but model_call_details is empty -- request context will be missing"
    );
    let calls = harness.transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(field(&calls[0].payload, "request"), &json("{}"));
}

#[tokio::test]
async fn moderate_response_raises_a_text_block_with_the_response_model_and_stash() {
    let harness = Harness::new(vec![ScriptedReply::json(
        r#"{"choices":[{"message":{"content":"I cannot help with that."}}]}"#,
    )]);
    let logging_obj = logging_object(r#"{"litellm_call_id":"call-1","model":"gpt-4o"}"#);
    let mut request_data = request_fields(&[
        ("model", text_field("request-model")),
        ("response", json(r#"{"model":"gpt-4.1-mini"}"#)),
    ]);
    let exception = block(
        moderate_response(
            &harness.logger,
            text_inputs(&["Tell me a secret."]),
            &mut request_data,
            Some(&logging_obj),
        )
        .await,
    );
    assert_eq!(exception.message, "I cannot help with that.");
    assert_eq!(exception.model, "gpt-4.1-mini");
    assert_eq!(exception.guardrail_name, "safety");
    assert!(matches!(
        request_data.logging_obj,
        LoggingObjectStash::Present(Some(_))
    ));
    assert!(matches!(
        exception.request_data.logging_obj,
        LoggingObjectStash::Present(Some(_))
    ));
    let details = logging_obj
        .model_call_details
        .lock()
        .expect("the details lock is not poisoned");
    assert_eq!(details.get("_rubrik_blocked"), Some(&Json::Bool(true)));
    drop(details);
    let calls = harness.transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        field(field(&calls[0].payload, "response"), "id"),
        &json(r#""call-1""#)
    );
}

#[tokio::test]
async fn moderate_response_raises_a_tool_block_with_an_unknown_model_without_a_logging_object() {
    let harness = Harness::new(vec![ScriptedReply::json(
        r#"{"choices":[{"message":{"content":"I'll check.\n\nTool call blocked by policy.","tool_calls":[{"id":"call_1"}]}}]}"#,
    )]);
    let inputs = GuardrailInputs {
        texts: vec!["I'll check.".to_owned()],
        tool_calls: vec![
            ToolCallSource::Raw(json(
                r#"{"id":"call_1","type":"function","function":{"name":"a","arguments":"{}"}}"#,
            )),
            ToolCallSource::Raw(json(
                r#"{"id":"call_2","type":"function","function":{"name":"b","arguments":"{}"}}"#,
            )),
        ],
        structured_messages: Vec::new(),
        model: None,
        tools: None,
    };
    let mut request_data = request_fields(&[
        ("litellm_call_id", text_field("call-1")),
        ("model", text_field("request-model")),
        ("tools", Json::List(Vec::new())),
    ]);
    let exception =
        block(moderate_response(&harness.logger, inputs, &mut request_data, None).await);
    assert_eq!(exception.message, "Tool call blocked by policy.");
    assert_eq!(exception.model, "unknown");
    assert!(matches!(
        request_data.logging_obj,
        LoggingObjectStash::Present(None)
    ));
    let errors = harness.logs.with_level(Level::ERROR);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].litellm_call_id.as_deref(), Some("call-1"));
    let calls = harness.transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        field(field(&calls[0].payload, "request"), "tools"),
        &json("[]")
    );
}

#[derive(Clone, Copy)]
enum ResponseCase {
    Absent,
    WithoutModel,
    WithModel(&'static str),
}

#[rstest]
#[case::response_model_wins(ResponseCase::WithModel("gpt-4.1-mini"), None, "gpt-4.1-mini")]
#[case::empty_response_model_is_unknown(ResponseCase::WithModel(""), None, "unknown")]
#[case::response_without_model_falls_back(ResponseCase::WithoutModel, Some("gpt-4o"), "gpt-4o")]
#[case::no_response_and_no_call_details_model_is_unknown(ResponseCase::Absent, None, "unknown")]
#[tokio::test]
async fn moderate_response_resolves_the_block_model(
    #[case] response: ResponseCase,
    #[case] call_details_model: Option<&str>,
    #[case] expected: &str,
) {
    let harness = Harness::new(vec![ScriptedReply::json(
        r#"{"choices":[{"message":{"content":"blocked"}}]}"#,
    )]);
    let logging_obj = call_details_model.map(|model| {
        logging_object(&format!(
            "{{\"litellm_call_id\":\"call-1\",\"model\":\"{model}\"}}"
        ))
    });
    let mut request_data = RequestData::default();
    match response {
        ResponseCase::Absent => {}
        ResponseCase::WithoutModel => {
            request_data
                .fields
                .insert("response".to_owned(), json("{}"));
        }
        ResponseCase::WithModel(model) => {
            request_data.fields.insert(
                "response".to_owned(),
                json(&format!("{{\"model\":\"{model}\"}}")),
            );
        }
    }
    let exception = block(
        moderate_response(
            &harness.logger,
            text_inputs(&["hello"]),
            &mut request_data,
            logging_obj.as_ref(),
        )
        .await,
    );
    assert_eq!(exception.model, expected);
}

#[tokio::test]
async fn moderate_response_reports_an_empty_choices_response_as_malformed() {
    let harness = Harness::new(vec![ScriptedReply::json(r#"{"choices": []}"#)]);
    let mut request_data = RequestData::default();
    let error = moderate_response(
        &harness.logger,
        text_inputs(&["hello"]),
        &mut request_data,
        None,
    )
    .await
    .expect_err("an empty choices list is malformed");
    assert!(matches!(
        error,
        ModerationError::Malformed(ResponseBlockError::MalformedResponse)
    ));
}

#[rstest]
#[case::choices_not_a_list(r#"{"choices": 3}"#, "choices")]
#[case::choice_not_an_object(r#"{"choices": [3]}"#, "choices[0]")]
#[case::message_not_an_object(r#"{"choices": [{"message": 3}]}"#, "choices[0].message")]
#[case::content_not_text(
    r#"{"choices": [{"message": {"content": 3}}]}"#,
    "choices[0].message.content"
)]
#[case::tool_calls_not_a_list(
    r#"{"choices": [{"message": {"tool_calls": 3}}]}"#,
    "choices[0].message.tool_calls"
)]
#[case::tool_call_not_an_object(
    r#"{"choices": [{"message": {"tool_calls": [3]}}]}"#,
    "choices[0].message.tool_calls"
)]
#[case::tool_call_id_not_text(r#"{"choices": [{"message": {"tool_calls": [{"id": 3}]}}]}"#, "id")]
fn service_response_rejects_a_wrong_typed_field(#[case] body: &str, #[case] expected: &str) {
    let Json::Object(fields) = json(body) else {
        panic!("the test body is an object");
    };
    let error = service_response(&fields).expect_err("the field type is rejected");
    assert!(matches!(error, ModerationError::FieldType { field } if field == expected));
}

#[rstest]
#[case::call_id(r#"{"litellm_call_id": 3}"#, "litellm_call_id")]
#[case::optional_params(r#"{"optional_params": "bad"}"#, "optional_params")]
#[tokio::test]
async fn moderate_response_rejects_a_wrong_typed_call_details_field(
    #[case] details: &str,
    #[case] expected: &str,
) {
    let harness = Harness::new(Vec::new());
    let logging_obj = logging_object(details);
    let mut request_data = RequestData::default();
    request_data
        .fields
        .insert("litellm_call_id".to_owned(), text_field("call-1"));
    let error = moderate_response(
        &harness.logger,
        text_inputs(&["hello"]),
        &mut request_data,
        Some(&logging_obj),
    )
    .await
    .expect_err("the field type is rejected");
    assert!(matches!(error, ModerationError::FieldType { field } if field == expected));
}

#[tokio::test]
async fn moderate_response_reports_a_clock_reading_past_the_unix_range() {
    let harness = Harness::with_clock(vec![], Arc::new(FarFutureClock));
    let mut request_data = RequestData::default();
    let error = moderate_response(
        &harness.logger,
        text_inputs(&["hello"]),
        &mut request_data,
        None,
    )
    .await
    .expect_err("the clock reading is out of range");
    assert!(matches!(error, ModerationError::Timestamp));
}

#[tokio::test]
async fn moderate_prompt_sends_flattened_messages_and_the_request_fields() {
    let harness = Harness::new(vec![ScriptedReply::json("{}")]);
    let inputs = GuardrailInputs {
        texts: Vec::new(),
        tool_calls: Vec::new(),
        structured_messages: vec![
            json(r#"{"role":"user","content":"hello"}"#),
            json(
                r#"{"role":"assistant","content":[{"type":"text","text":"part"}],"tool_calls":[{"function":{"arguments":"{\"x\":1}"}}]}"#,
            ),
        ],
        model: Some("gpt-4o".to_owned()),
        tools: Some(Json::List(Vec::new())),
    };
    let mut request_data = request_fields(&[
        ("litellm_call_id", text_field("call-1")),
        ("model", text_field("request-model")),
        ("user", text_field("u1")),
        ("correlation_key", text_field("ck")),
    ]);
    let returned = moderate_prompt(&harness.logger, inputs.clone(), &mut request_data, None)
        .await
        .expect("an empty choices answer is a pass");
    assert_same_inputs(&returned, &inputs);
    let calls = harness.transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].payload, expected_prompt_payload());
}

#[tokio::test]
async fn moderate_prompt_calls_the_webhook_for_text_only_requests() {
    let harness = Harness::new(vec![ScriptedReply::json("{}")]);
    let inputs = text_inputs(&["one", "", "two"]);
    let mut request_data = RequestData::default();
    let returned = moderate_prompt(&harness.logger, inputs.clone(), &mut request_data, None)
        .await
        .expect("an empty choices answer is a pass");
    assert_same_inputs(&returned, &inputs);
    let calls = harness.transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].payload, json(r#"{"model":"","messages":[]}"#));
}

#[rstest]
#[case::no_inputs(Vec::new(), Vec::new())]
#[case::only_empty_texts(
    vec![String::new(), String::new()],
    Vec::new()
)]
#[tokio::test]
async fn moderate_prompt_returns_inputs_without_a_call_when_nothing_is_moderated(
    #[case] texts: Vec<String>,
    #[case] structured_messages: Vec<Json>,
) {
    let harness = Harness::new(Vec::new());
    let inputs = GuardrailInputs {
        texts,
        tool_calls: Vec::new(),
        structured_messages,
        model: None,
        tools: None,
    };
    let mut request_data = RequestData::default();
    let returned = moderate_prompt(&harness.logger, inputs.clone(), &mut request_data, None)
        .await
        .expect("nothing to moderate");
    assert_same_inputs(&returned, &inputs);
    assert!(harness.transport.calls().is_empty());
}

#[rstest]
#[case::input_model_wins(Some("gpt-4o"), Some("request-model"), "gpt-4o")]
#[case::request_model_falls_back(None, Some("request-model"), "request-model")]
#[case::no_model_is_unknown(None, None, "unknown")]
// docs/compatibility.md: an explicit "" is used as given; LiteLLM's or-chain would fall through
#[case::explicit_empty_input_model_is_kept(Some(""), Some("request-model"), "")]
#[tokio::test]
async fn moderate_prompt_raises_a_block_with_the_resolved_model(
    #[case] input_model: Option<&str>,
    #[case] request_model: Option<&str>,
    #[case] expected: &str,
) {
    let harness = Harness::new(vec![ScriptedReply::json(
        r#"{"choices":[{"message":{"content":"Prompt blocked by policy."}}]}"#,
    )]);
    let inputs = GuardrailInputs {
        texts: Vec::new(),
        tool_calls: Vec::new(),
        structured_messages: vec![json(r#"{"role":"user","content":"hello"}"#)],
        model: input_model.map(str::to_owned),
        tools: None,
    };
    let mut request_data = RequestData::default();
    if let Some(model) = request_model {
        request_data
            .fields
            .insert("model".to_owned(), text_field(model));
    }
    let exception = block(moderate_prompt(&harness.logger, inputs, &mut request_data, None).await);
    assert_eq!(exception.message, "Prompt blocked by policy.");
    assert_eq!(exception.model, expected);
    assert_eq!(exception.guardrail_name, "safety");
    assert!(matches!(
        request_data.logging_obj,
        LoggingObjectStash::Present(None)
    ));
}

#[tokio::test]
async fn moderate_prompt_returns_inputs_when_the_service_passes_through() {
    let harness = Harness::new(vec![ScriptedReply::json(r#"{"choices": []}"#)]);
    let inputs = GuardrailInputs {
        texts: Vec::new(),
        tool_calls: Vec::new(),
        structured_messages: vec![json(r#"{"role":"user","content":"hello"}"#)],
        model: None,
        tools: None,
    };
    let mut request_data = RequestData::default();
    let returned = moderate_prompt(&harness.logger, inputs.clone(), &mut request_data, None)
        .await
        .expect("an empty choices answer is a pass");
    assert_same_inputs(&returned, &inputs);
    assert!(matches!(
        request_data.logging_obj,
        LoggingObjectStash::Missing
    ));
}

#[tokio::test]
async fn stash_block_context_marks_the_logging_object_and_stashes_it() {
    let _harness = Harness::new(Vec::new());
    let logging_obj: Arc<LoggingObject> = logging_object(r#"{"model":"gpt-4o"}"#);
    let mut request_data = RequestData::default();
    stash_block_context(Some(&logging_obj), &mut request_data);
    let details = logging_obj
        .model_call_details
        .lock()
        .expect("the details lock is not poisoned");
    assert_eq!(details.get("_rubrik_blocked"), Some(&Json::Bool(true)));
    drop(details);
    let LoggingObjectStash::Present(Some(stashed)) = &request_data.logging_obj else {
        panic!(
            "the logging object is stashed, got {:?}",
            request_data.logging_obj
        );
    };
    assert!(Arc::ptr_eq(stashed, &logging_obj));
}

#[rstest]
#[case::with_a_call_id(Some("call-9"), "call-9")]
#[case::without_a_call_id(None, "None")]
#[tokio::test]
async fn stash_block_context_without_a_logging_object_stashes_none_and_logs(
    #[case] call_id: Option<&str>,
    #[case] logged_call_id: &str,
) {
    let harness = Harness::new(Vec::new());
    let mut request_data = RequestData::default();
    if let Some(present) = call_id {
        request_data
            .fields
            .insert("litellm_call_id".to_owned(), text_field(present));
    }
    stash_block_context(None, &mut request_data);
    assert!(matches!(
        request_data.logging_obj,
        LoggingObjectStash::Present(None)
    ));
    let errors = harness.logs.with_level(Level::ERROR);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].litellm_call_id.as_deref(), Some(logged_call_id));
    assert!(
        errors[0]
            .message
            .contains("moderation block fired with logging_obj=None")
    );
}

#[tokio::test]
async fn apply_guardrail_fails_open_on_a_response_transport_error() {
    let harness = Harness::new(vec![ScriptedReply::network()]);
    let inputs = text_inputs(&["hello"]);
    let mut request_data = RequestData::default();
    let returned = apply_guardrail(
        &harness.logger,
        inputs.clone(),
        &mut request_data,
        ModerationInputType::Response,
        None,
    )
    .await
    .expect("a transport failure fails open");
    assert_same_inputs(&returned, &inputs);
    let errors = harness.logs.with_level(Level::ERROR);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].severity, None);
}

#[tokio::test]
async fn apply_guardrail_fails_open_on_a_prompt_non_object_response() {
    let harness = Harness::new(vec![ScriptedReply::json("[]")]);
    let inputs = GuardrailInputs {
        texts: Vec::new(),
        tool_calls: Vec::new(),
        structured_messages: vec![json(r#"{"role":"user","content":"hello"}"#)],
        model: None,
        tools: None,
    };
    let mut request_data = RequestData::default();
    let returned = apply_guardrail(
        &harness.logger,
        inputs.clone(),
        &mut request_data,
        ModerationInputType::Request,
        None,
    )
    .await
    .expect("a non-object answer fails open");
    assert_same_inputs(&returned, &inputs);
    let errors = harness.logs.with_level(Level::ERROR);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].severity, None);
}
