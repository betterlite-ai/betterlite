use std::convert::Infallible;

use betterlite_models::decisions::Json;
use http::HeaderValue;
use indexmap::IndexMap;

use crate::response::error::{DecodeError, ResponseError};
use crate::response::get_error_class;
use crate::response::get_prefix_prompt;
use crate::response::wire::ResponseEnvelope;

use super::fixture::{
    CALLER_ID, MODEL, NOW, Outcome, Scenario, TEXT, UPSTREAM_ID, body, logged, parsed,
    response_headers,
};

fn object(value: Option<&Json>) -> &IndexMap<String, Json> {
    match value {
        Some(Json::Object(fields)) => fields,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => {
            panic!("the fixture should hold an object")
        }
    }
}

fn content_of(envelope: &ResponseEnvelope) -> Option<&str> {
    envelope.choices[0].message.content.as_deref()
}

#[test]
fn case_1_message_envelope() {
    let source = body(
        r#"[{"type":"text","text":"Hello"},{"type":"thinking","thinking":"plan"},{"type":"tool_use","id":"toolu_1","name":"get_weather","input":{"city":"Paris"}}]"#,
        "end_turn",
    );
    let Outcome {
        result, details, ..
    } = Scenario::new(&source).run();
    let envelope = result.expect("the envelope should be built");
    assert_envelope_identity(&envelope);
    assert_message(&envelope);
    assert_tool_calls(&envelope);
    assert_usage(&envelope);
    assert_hidden_params(&envelope);
    assert_eq!(
        logged(&details),
        &Json::Text(source),
        "the raw reply is logged"
    );
    speed_reaches_usage();
}

fn assert_envelope_identity(envelope: &ResponseEnvelope) {
    assert_eq!(envelope.id, CALLER_ID);
    assert_ne!(envelope.id, UPSTREAM_ID);
    assert_eq!(envelope.object, "chat.completion");
    assert_eq!(envelope.created, NOW);
    assert_eq!(envelope.model.as_deref(), Some(MODEL));
    assert_eq!(envelope.system_fingerprint, None);
    assert_eq!(envelope.choices.len(), 1);
    assert_eq!(envelope.choices[0].index, 0);
    assert_eq!(envelope.choices[0].finish_reason.as_deref(), Some("stop"));
}

fn assert_message(envelope: &ResponseEnvelope) {
    let message = &envelope.choices[0].message;
    assert_eq!(message.role, "assistant");
    assert_eq!(message.content.as_deref(), Some("Hello"));
    assert_eq!(message.reasoning_content.as_deref(), Some("plan"));
    let thinking = message.thinking_blocks.as_ref().expect("thinking blocks");
    assert_eq!(
        object(thinking.first()).get("thinking"),
        Some(&Json::Text("plan".to_owned()))
    );
}

fn assert_tool_calls(envelope: &ResponseEnvelope) {
    let calls = envelope.choices[0]
        .message
        .tool_calls
        .as_ref()
        .expect("tool calls");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id.as_deref(), Some("toolu_1"));
    assert_eq!(calls[0].function.name.as_deref(), Some("get_weather"));
    assert_eq!(
        calls[0].function.arguments.as_deref(),
        Some(r#"{"city": "Paris"}"#)
    );
    assert_eq!(calls[0].index, 2);
}

fn assert_usage(envelope: &ResponseEnvelope) {
    let usage = envelope.usage.as_ref().expect("usage");
    let counts = (
        usage.prompt_tokens,
        usage.completion_tokens,
        usage.total_tokens,
        usage.completion_tokens_details.reasoning,
        usage.completion_tokens_details.text,
    );
    assert_eq!(counts, (18, 4, 22, Some(2), Some(2)));
}

fn assert_hidden_params(envelope: &ResponseEnvelope) {
    assert_eq!(envelope.hidden_params.len(), 2);
    assert!(!envelope.hidden_params.contains_key("original_response"));
    let headers = object(envelope.hidden_params.get("additional_headers"));
    assert_eq!(
        headers.get("llm_provider-x-request-id"),
        Some(&Json::Text("req-b580".to_owned()))
    );
    assert_eq!(
        headers.get("x-ratelimit-limit-requests"),
        Some(&Json::Text("100".to_owned()))
    );
    let provider = object(envelope.hidden_params.get("provider_specific_fields"));
    assert_eq!(provider.get("citations"), Some(&Json::Null));
}

fn speed_reaches_usage() {
    let envelope = Scenario::new(&body(TEXT, "end_turn"))
        .optional_params(r#"{"speed":"fast"}"#)
        .run()
        .envelope();
    assert_eq!(
        envelope.usage.as_ref().expect("usage").speed.as_deref(),
        Some("fast")
    );
    let error = Scenario::new(&body(TEXT, "end_turn"))
        .optional_params(r#"{"speed":5}"#)
        .run()
        .error();
    assert!(
        matches!(error, ResponseError::Parameter { field } if field == "speed"),
        "{error}"
    );
}

#[test]
fn case_2_finish_reasons() {
    stop_reasons();
    unknown_reason_warns();
    null_reason_warns();
    required_fields_stay_structural();
    missing_choice_stays_structural();
}

fn stop_reasons() {
    assert_finish("end_turn", "stop");
    assert_finish("stop_sequence", "stop");
    assert_finish("max_tokens", "length");
    assert_finish("tool_use", "tool_calls");
    assert_finish("refusal", "content_filter");
}

fn assert_finish(stop_reason: &str, expected: &str) {
    let envelope = Scenario::new(&body(TEXT, stop_reason)).run().envelope();
    assert_eq!(
        envelope.choices[0].finish_reason.as_deref(),
        Some(expected),
        "{stop_reason}"
    );
}

fn finished_with(source: &str) -> (Option<String>, Vec<String>) {
    let Outcome {
        result, warnings, ..
    } = Scenario::new(source).run();
    let envelope = result.expect("the response still finishes");
    (envelope.choices[0].finish_reason.clone(), warnings)
}

fn unknown_reason_warns() {
    let (reason, warnings) = finished_with(&body(TEXT, "surprise"));
    assert_eq!(reason.as_deref(), Some("stop"));
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("Unmapped finish_reason 'surprise'")),
        "{warnings:?}"
    );
}

fn null_reason_warns() {
    let source = format!(
        r#"{{"model":"{MODEL}","content":{TEXT},"stop_reason":null,"usage":{{"input_tokens":1,"output_tokens":1}}}}"#
    );
    let (reason, warnings) = finished_with(&source);
    assert_eq!(reason.as_deref(), Some("stop"));
    assert!(!warnings.is_empty(), "the fallback warns");
}

fn required_fields_stay_structural() {
    let no_stop = format!(r#"{{"model":"{MODEL}","content":{TEXT},"usage":{{"input_tokens":1}}}}"#);
    let no_model =
        format!(r#"{{"content":{TEXT},"stop_reason":"end_turn","usage":{{"input_tokens":1}}}}"#);
    let no_usage = format!(r#"{{"model":"{MODEL}","content":{TEXT},"stop_reason":"end_turn"}}"#);
    assert!(matches!(
        Scenario::new(&no_stop).run().error(),
        ResponseError::MissingField("stop_reason")
    ));
    assert!(matches!(
        Scenario::new(&no_model).run().error(),
        ResponseError::MissingField("model")
    ));
    assert!(matches!(
        Scenario::new(&no_usage).run().error(),
        ResponseError::MissingField("usage")
    ));
    assert!(matches!(
        Scenario::new(r#"{"model":"m","stop_reason":"end_turn","usage":{}}"#)
            .run()
            .error(),
        ResponseError::Decode(DecodeError::MissingContent)
    ));
    assert!(matches!(
        Scenario::new(r#"{"model":"m","content":"text","stop_reason":"end_turn","usage":{}}"#)
            .run()
            .error(),
        ResponseError::Decode(DecodeError::ContentType)
    ));
}

fn missing_choice_stays_structural() {
    let error = Scenario::new(&body(TEXT, "end_turn"))
        .without_choices()
        .run()
        .error();
    assert!(matches!(error, ResponseError::MissingChoice), "{error}");
}

#[test]
fn case_3_reverse_tool_name() {
    let source = body(
        r#"[{"type":"text","text":""},{"type":"tool_use","id":"toolu_a","name":"sanitized_name","input":{"q":"x"}},{"type":"tool_use","id":"toolu_b","name":"kept","input":{}}]"#,
        "tool_use",
    );
    renames_mapped_calls(&source);
    unmapped_maps_leave_names_alone(&source);
    rejects_a_non_text_value(&source);
}

fn first_tool_name(envelope: &ResponseEnvelope) -> Option<&str> {
    envelope.choices[0].message.tool_calls.as_ref()?[0]
        .function
        .name
        .as_deref()
}

fn renames_mapped_calls(source: &str) {
    let reverse = r#"{"_anthropic_tool_name_map": {"sanitized_name": "caller.name"}}"#;
    let envelope = Scenario::new(source)
        .litellm_params(reverse)
        .run()
        .envelope();
    let tool_calls = envelope.choices[0]
        .message
        .tool_calls
        .as_ref()
        .expect("tool calls");
    assert_eq!(tool_calls[0].function.name.as_deref(), Some("caller.name"));
    assert_eq!(tool_calls[0].id.as_deref(), Some("toolu_a"));
    assert_eq!(
        tool_calls[0].function.arguments.as_deref(),
        Some(r#"{"q": "x"}"#)
    );
    assert_eq!(tool_calls[0].index, 1);
    assert_eq!(tool_calls[1].function.name.as_deref(), Some("kept"));
}

fn unmapped_maps_leave_names_alone(source: &str) {
    let unrelated = r#"{"_anthropic_tool_name_map": {"other": "caller.other"}}"#;
    let kept = Scenario::new(source)
        .litellm_params(unrelated)
        .run()
        .envelope();
    assert_eq!(first_tool_name(&kept), Some("sanitized_name"));
    let not_a_map = r#"{"_anthropic_tool_name_map": "caller.name"}"#;
    let ignored = Scenario::new(source)
        .litellm_params(not_a_map)
        .run()
        .envelope();
    assert_eq!(first_tool_name(&ignored), Some("sanitized_name"));
}

fn rejects_a_non_text_value(source: &str) {
    let bad_value = r#"{"_anthropic_tool_name_map": {"sanitized_name": 5}}"#;
    let error = Scenario::new(source)
        .litellm_params(bad_value)
        .run()
        .error();
    assert!(
        matches!(&error, ResponseError::Parameter { field } if *field == "_anthropic_tool_name_map"),
        "{error}"
    );
}

#[test]
fn case_4_json_mode_message() {
    pure_json_message();
    mixed_json_text();
    tool_only_message_has_no_content();
    the_reverse_map_reaches_both_projections();
    two_internal_json_tools_keep_the_first();
}

fn pure_json_message() {
    let source = body(
        r#"[{"type":"text","text":"ignored"},{"type":"thinking","thinking":"plan"},{"type":"tool_use","id":"j","name":"json_tool_call","input":{"values":{"a":1}}}]"#,
        "max_tokens",
    );
    let envelope = Scenario::new(&source).json_mode(true).run().envelope();
    let choice = &envelope.choices[0];
    let message = &choice.message;
    assert_eq!(message.content.as_deref(), Some(r#"{"a": 1}"#));
    assert_eq!(message.tool_calls, None);
    assert_eq!(message.thinking_blocks, None);
    assert_eq!(message.reasoning_content, None);
    assert_eq!(message.provider_specific_fields, None);
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    let provider = object(envelope.hidden_params.get("provider_specific_fields"));
    let thinking = provider.get("thinking_blocks").expect("hidden thinking");
    assert!(matches!(thinking, Json::List(blocks) if blocks.len() == 1));
}

fn mixed_json_text() {
    let source = body(
        r#"[{"type":"text","text":"pre:"},{"type":"tool_use","id":"j","name":"json_tool_call","input":{"values":"x"}},{"type":"tool_use","id":"r","name":"run","input":{}}]"#,
        "tool_use",
    );
    let envelope = Scenario::new(&source).json_mode(true).run().envelope();
    let choice = &envelope.choices[0];
    assert_eq!(choice.message.content.as_deref(), Some(r#"pre:"x""#));
    let tool_calls = choice.message.tool_calls.as_ref().expect("real tool");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name.as_deref(), Some("run"));
    assert_eq!(choice.finish_reason.as_deref(), Some("tool_calls"));
}

fn tool_only_message_has_no_content() {
    let source = body(
        r#"[{"type":"tool_use","id":"r","name":"run","input":{}}]"#,
        "tool_use",
    );
    let envelope = Scenario::new(&source).run().envelope();
    assert_eq!(content_of(&envelope), None);
    assert!(
        envelope.choices[0].message.tool_calls.is_some(),
        "the tool call stays"
    );
}

fn the_reverse_map_reaches_both_projections() {
    let renamed_internal = body(
        r#"[{"type":"text","text":"kept"},{"type":"tool_use","id":"j","name":"json_tool_call","input":{"values":1}}]"#,
        "tool_use",
    );
    let first = Scenario::new(&renamed_internal)
        .json_mode(true)
        .litellm_params(r#"{"_anthropic_tool_name_map": {"json_tool_call": "json.tool.call"}}"#)
        .run()
        .envelope();
    assert_eq!(first.choices[0].message.content.as_deref(), Some("kept"));
    assert_eq!(first_tool_name(&first), Some("json.tool.call"));
    assert_eq!(
        first.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
    let renamed_real = body(
        r#"[{"type":"text","text":"pre"},{"type":"tool_use","id":"j","name":"json_tool_call","input":{"values":1}},{"type":"tool_use","id":"r","name":"sanitized","input":{}}]"#,
        "tool_use",
    );
    let second = Scenario::new(&renamed_real)
        .json_mode(true)
        .litellm_params(r#"{"_anthropic_tool_name_map": {"sanitized": "caller.tool"}}"#)
        .run()
        .envelope();
    assert_eq!(second.choices[0].message.content.as_deref(), Some("pre1"));
    assert_eq!(first_tool_name(&second), Some("caller.tool"));
    assert_eq!(
        second.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
}

fn two_internal_json_tools_keep_the_first() {
    let source = body(
        r#"[{"type":"text","text":"ignored"},{"type":"tool_use","id":"j1","name":"json_tool_call","input":{"values":{"a":1}}},{"type":"tool_use","id":"j2","name":"json_tool_call","input":{"values":{"b":2}}}]"#,
        "max_tokens",
    );
    let envelope = Scenario::new(&source).json_mode(true).run().envelope();
    let choice = &envelope.choices[0];
    assert_eq!(choice.message.content.as_deref(), Some(r#"{"a": 1}"#));
    assert_eq!(choice.message.tool_calls, None);
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn case_5_prefix_prompt() {
    assert_eq!(get_prefix_prompt::<Infallible>(&[]).unwrap(), None);
    let assistant = parsed::<Vec<Json>>(r#"[{"role":"assistant","content":"Hi","prefix":true}]"#);
    assert_eq!(
        get_prefix_prompt::<Infallible>(&assistant).unwrap(),
        Some("Hi".to_owned())
    );
    let empty = parsed::<Vec<Json>>(r#"[{"role":"assistant","content":"","prefix":true}]"#);
    assert_eq!(
        get_prefix_prompt::<Infallible>(&empty).unwrap(),
        Some(String::new())
    );
    let blocks = parsed::<Vec<Json>>(
        r#"[{"role":"assistant","content":[{"type":"text","text":"Hi"}],"prefix":true}]"#,
    );
    assert_eq!(get_prefix_prompt::<Infallible>(&blocks).unwrap(), None);
    let disabled = parsed::<Vec<Json>>(r#"[{"role":"assistant","content":"Hi","prefix":false}]"#);
    assert_eq!(get_prefix_prompt::<Infallible>(&disabled).unwrap(), None);

    prefixed();
    earlier_messages_are_ignored();
    existing_prefix_is_not_duplicated();
    the_flag_disables_the_prefix();
    an_empty_prefix_changes_nothing();
    a_non_assistant_prefix_is_ignored();
}

fn a_non_assistant_prefix_is_ignored() {
    let source = body(r#"[{"type":"text","text":"world"}]"#, "end_turn");
    let messages = r#"[{"role":"user","content":"Pre ","prefix":true}]"#;
    let envelope = Scenario::new(&source).messages(messages).run().envelope();
    assert_eq!(content_of(&envelope), Some("world"));
}

fn prefixed() {
    let source = body(r#"[{"type":"text","text":"world"}]"#, "end_turn");
    let messages = r#"[{"role":"user","content":"say hi"},{"role":"assistant","content":"Pre ","prefix":true}]"#;
    let envelope = Scenario::new(&source).messages(messages).run().envelope();
    assert_eq!(content_of(&envelope), Some("Pre world"));
}

fn earlier_messages_are_ignored() {
    let source = body(r#"[{"type":"text","text":"world"}]"#, "end_turn");
    let messages =
        r#"[{"role":"assistant","content":"Early ","prefix":true},{"role":"user","content":"go"}]"#;
    let envelope = Scenario::new(&source).messages(messages).run().envelope();
    assert_eq!(content_of(&envelope), Some("world"));
}

fn existing_prefix_is_not_duplicated() {
    let source = body(r#"[{"type":"text","text":"Pre world"}]"#, "end_turn");
    let messages = r#"[{"role":"assistant","content":"Pre ","prefix":true}]"#;
    let envelope = Scenario::new(&source).messages(messages).run().envelope();
    assert_eq!(content_of(&envelope), Some("Pre world"));
}

fn the_flag_disables_the_prefix() {
    let source = body(r#"[{"type":"text","text":"world"}]"#, "end_turn");
    let messages = r#"[{"role":"assistant","content":"Pre ","prefix":true}]"#;
    let envelope = Scenario::new(&source)
        .messages(messages)
        .with_disabled_prefix()
        .run()
        .envelope();
    assert_eq!(content_of(&envelope), Some("world"));
}

fn an_empty_prefix_changes_nothing() {
    let source = body(r#"[{"type":"text","text":"world"}]"#, "end_turn");
    let messages = r#"[{"role":"assistant","content":"","prefix":true}]"#;
    let envelope = Scenario::new(&source).messages(messages).run().envelope();
    assert_eq!(content_of(&envelope), Some("world"));
}

#[test]
fn case_6_malformed_response_json() {
    malformed_body();
    truncated_body();
    invalid_utf8_body();
    error_class_is_a_pass_through();
}

fn malformed_body() {
    let Outcome {
        result, details, ..
    } = Scenario::new("not json").status(200).run();
    let error = result.expect_err("a text body cannot decode");
    let ResponseError::Anthropic {
        message,
        status,
        headers,
    } = error
    else {
        panic!("a json failure is an AnthropicError");
    };
    assert_eq!(status, 200);
    assert!(
        message.starts_with("Unable to get json response - "),
        "{message}"
    );
    assert!(
        message.ends_with("Original Response: not json"),
        "{message}"
    );
    assert_eq!(
        headers.get("x-request-id"),
        Some(&HeaderValue::from_static("req-b580"))
    );
    assert_eq!(logged(&details), &Json::Text("not json".to_owned()));
    assert_eq!(
        details.get("log_event_type"),
        Some(&Json::Text("post_api_call".to_owned()))
    );
}

fn truncated_body() {
    let Outcome { result, .. } = Scenario::new(r#"{"content":[{"type":"text""#).run();
    assert!(
        matches!(result, Err(ResponseError::Anthropic { status: 200, .. })),
        "a truncated body raises the same error"
    );
}

fn invalid_utf8_body() {
    let outcome = Scenario::new("").raw(&[b'{', 0xFF, b'}']).run();
    assert!(
        matches!(
            outcome.result,
            Err(ResponseError::Anthropic { status: 200, .. })
        ),
        "invalid UTF-8 reaches the same branch"
    );
}

fn error_class_is_a_pass_through() {
    let error = get_error_class::<Infallible>("slow down".to_owned(), 529, response_headers());
    let ResponseError::Anthropic {
        message,
        status,
        headers,
    } = error
    else {
        panic!("the hook builds an AnthropicError");
    };
    assert_eq!(message, "slow down");
    assert_eq!(status, 529);
    assert_eq!(headers.len(), 3);
}

#[test]
fn case_7_upstream_anthropic_error() {
    let source = r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#;
    let error = Scenario::new(source).status(200).run().error();
    let ResponseError::Anthropic {
        message,
        status,
        headers,
    } = error
    else {
        panic!("an error key raises AnthropicError");
    };
    assert_eq!(status, 200);
    assert!(message.contains("rate_limit_error"), "{message}");
    assert_eq!(
        headers.get("x-request-id"),
        Some(&HeaderValue::from_static("req-b580"))
    );
    assert_null_error();
    assert_empty_error();
    assert_error_key_wins_over_content();
}

fn assert_null_error() {
    let error = Scenario::new(r#"{"error":null,"content":"text"}"#)
        .run()
        .error();
    assert!(
        matches!(&error, ResponseError::Anthropic { message, .. } if message == "null"),
        "{error}"
    );
}

fn assert_empty_error() {
    let error = Scenario::new(r#"{"error":""}"#).run().error();
    assert!(
        matches!(&error, ResponseError::Anthropic { message, .. } if message == r#""""#),
        "{error}"
    );
}

fn assert_error_key_wins_over_content() {
    let error = Scenario::new(r#"{"error":"boom","content":5}"#)
        .run()
        .error();
    assert!(
        matches!(&error, ResponseError::Anthropic { status: 200, .. }),
        "{error}"
    );
}
