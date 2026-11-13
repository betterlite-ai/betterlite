#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::sdk::response::{
    ChoiceMode, IncompleteReason, NamedChoice, NamedKind, ResponseTools, ToolChoice,
};
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiTruncation};
use betterlite_openai::api::responses_shapes::{
    FunctionCallStatus, OutputCallStatus, OutputImageGenerationCall,
};
use rstest::rstest;
use serde::Serialize;

use super::assemble_response;
use super::output::interpreter_results;
use super::records::{AssembledResponse, ChatCompletionResponse, RequestFields};
use super::status::{finish_status, incomplete_details};
use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_chat_output::error::ChatOutputError;

fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn serialized<Item>(value: &Item) -> Json
where
    Item: Serialize,
{
    parse(&serde_json::to_string(value).unwrap())
}

fn field<'value>(value: &'value Json, name: &str) -> &'value Json {
    let Json::Object(fields) = value else {
        panic!("a JSON object was expected");
    };
    fields.get(name).unwrap()
}

fn element(value: &Json, position: usize) -> &Json {
    let Json::List(items) = value else {
        panic!("a JSON list was expected");
    };
    &items[position]
}

fn assemble_with(chat_text: &str, request_text: &str) -> AssembledResponse {
    let chat: ChatCompletionResponse = serde_json::from_str(chat_text).unwrap();
    let request: RequestFields = serde_json::from_str(request_text).unwrap();
    let mut ids = 0_u32;
    assemble_response(&Json::Null, &request, &chat, ApiLimits::default(), || {
        ids = ids.saturating_add(1);
        format!("{ids}")
    })
    .unwrap()
}

fn assemble(chat_text: &str) -> AssembledResponse {
    assemble_with(chat_text, "{}")
}

fn failure(chat_text: &str) -> ChatOutputError {
    let chat: ChatCompletionResponse = serde_json::from_str(chat_text).unwrap();
    assemble_response(
        &Json::Null,
        &RequestFields {
            tool_choice: None,
            tools: None,
        },
        &chat,
        ApiLimits::default(),
        || "id".to_owned(),
    )
    .unwrap_err()
}

fn message_items(response: &AssembledResponse) -> Vec<&ResponseOutputItem> {
    response
        .response
        .output
        .iter()
        .filter(|item| {
            matches!(item, ResponseOutputItem::Message(message) if message.r#type == "message")
        })
        .collect()
}

const CHAT_A: &str = r#"{"id":"chat_a","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi"}}]}"#;

#[test]
fn response_defaults() {
    let response = assemble(CHAT_A);
    let body = &response.response;
    assert_eq!(body.id, "chat_a");
    assert_eq!(body.created_at, 1);
    assert_eq!(body.model.as_deref(), Some("m"));
    assert_eq!(body.object.as_deref(), Some("response"));
    assert_eq!(body.status.as_deref(), Some("completed"));
    assert_eq!(body.metadata, Some(BTreeMap::new()));
    assert_eq!(body.parallel_tool_calls, Some(false));
    assert_eq!(body.tool_choice, Some(ToolChoice::Mode(ChoiceMode::Auto)));
    assert_eq!(body.tools, Some(ResponseTools::Definitions(Vec::new())));
    assert_eq!(body.reasoning, None);
    assert_eq!(body.text, Some(BTreeMap::new()));
    assert!(body.extra.is_empty());
    assert!(response.hidden_params.is_empty());
    assert!(response.cache_writes.is_empty());
}

#[test]
fn optional_response_fields_default_to_null() {
    let body = assemble(CHAT_A).response;
    assert_eq!(body.error, None);
    assert_eq!(body.instructions, None);
    assert_eq!(body.incomplete_details, None);
    assert_eq!(body.top_p, None);
    assert_eq!(body.max_output_tokens, None);
    assert_eq!(body.previous_response_id, None);
    assert_eq!(body.truncation, None);
    assert_eq!(body.user, None);
    assert_eq!(body.store, None);
}

#[test]
fn usage_defaults_to_zero() {
    let body = assemble(CHAT_A).response;
    let usage = body.usage.as_ref().unwrap();
    assert_eq!(usage.input_tokens, 0);
    assert_eq!(usage.output_tokens, 0);
    assert_eq!(usage.total_tokens, 0);
    assert_eq!(usage.cost, None);
    assert_eq!(usage.input_tokens_details, None);
    assert_eq!(usage.output_tokens_details, None);
}

#[test]
fn default_temperature_serializes_as_a_float() {
    let body = serialized(&assemble(CHAT_A).response);
    assert_eq!(field(&body, "temperature"), &parse("0.0"));
    assert_eq!(field(&body, "tool_choice"), &parse("\"auto\""));
}

#[test]
fn explicit_nulls_override_the_defaults() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":null,"message":{"role":"assistant","content":"hi"}}],
            "metadata":null,"parallel_tool_calls":null,"temperature":null,"tools":null,"top_p":null,
            "instructions":null,"max_output_tokens":null,"previous_response_id":null,"truncation":null,"user":null,"error":null}"#,
    );
    let body = &response.response;
    assert_eq!(body.status.as_deref(), Some("completed"));
    assert_eq!(body.metadata, None);
    assert_eq!(body.parallel_tool_calls, None);
    assert_eq!(body.temperature, None);
    assert_eq!(body.tools, None);
}

#[test]
fn existing_attributes_override_the_defaults() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi"}}],
            "metadata":{"k":1},"parallel_tool_calls":true,"temperature":0.7,"top_p":0.9,
            "instructions":"be nice","max_output_tokens":12,"previous_response_id":"prev","truncation":"auto","user":"me",
            "error":{"message":"e"},"tools":[{"type":"function","name":"f","parameters":{}}]}"#,
    );
    let body = &response.response;
    assert_eq!(
        body.metadata.as_ref().unwrap().get("k"),
        Some(&Json::Number(serde_json::Number::from(1_i32)))
    );
    assert_eq!(body.parallel_tool_calls, Some(true));
    assert_eq!(body.temperature, Some("0.7".parse().unwrap()));
    assert_eq!(body.top_p, Some("0.9".parse().unwrap()));
    assert_eq!(body.instructions.as_deref(), Some("be nice"));
    assert_eq!(body.max_output_tokens, Some(12));
    assert_eq!(body.previous_response_id.as_deref(), Some("prev"));
    assert_eq!(body.truncation, Some(ResponsesApiTruncation::Auto));
    assert_eq!(body.user.as_deref(), Some("me"));
    assert_eq!(
        body.error.as_ref().unwrap().get("message"),
        Some(&Json::Text("e".to_owned()))
    );
    let tools = serialized(body.tools.as_ref().unwrap());
    assert_eq!(
        element(&tools, 0),
        &parse(
            r#"{"type":"function","name":"f","parameters":{},"strict":null,"allowed_callers":null,"defer_loading":null,"description":null,"output_schema":null}"#
        )
    );
}

#[rstest]
#[case::stop(Some("stop"), "completed")]
#[case::tool_calls(Some("tool_calls"), "completed")]
#[case::function_call(Some("function_call"), "completed")]
#[case::unknown(Some("something_new"), "completed")]
#[case::absent(None, "completed")]
#[case::length(Some("length"), "incomplete")]
#[case::content_filter(Some("content_filter"), "incomplete")]
#[case::refusal(Some("refusal"), "incomplete")]
fn finish_reasons_map_to_status(#[case] reason: Option<&str>, #[case] expected: &str) {
    assert_eq!(finish_status(reason), expected);
}

#[rstest]
#[case::length(Some("length"), Some(IncompleteReason::MaxOutputTokens))]
#[case::content_filter(Some("content_filter"), Some(IncompleteReason::ContentFilter))]
#[case::refusal(Some("refusal"), Some(IncompleteReason::ContentFilter))]
#[case::stop(Some("stop"), None)]
#[case::absent(None, None)]
fn incomplete_details_follow_the_finish_reason(
    #[case] reason: Option<&str>,
    #[case] expected: Option<IncompleteReason>,
) {
    let details = incomplete_details(reason, None);
    assert_eq!(details.and_then(|found| found.reason), expected);
}

#[test]
fn existing_incomplete_details_win() {
    let existing = incomplete_details(Some("content_filter"), None);
    let details = incomplete_details(Some("length"), existing);
    assert_eq!(
        details.and_then(|found| found.reason),
        Some(IncompleteReason::ContentFilter)
    );
}

#[test]
fn the_first_choice_controls_status_while_items_use_their_own() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[
            {"finish_reason":"length","message":{"role":"assistant","content":"cut"}},
            {"finish_reason":"stop","message":{"role":"assistant","content":"done"}}]}"#,
    );
    assert_eq!(response.response.status.as_deref(), Some("incomplete"));
    assert_eq!(
        response
            .response
            .incomplete_details
            .clone()
            .and_then(|found| found.reason),
        Some(IncompleteReason::MaxOutputTokens)
    );
    let items = message_items(&response);
    assert_eq!(items.len(), 2);
    for (item, status) in items.iter().zip(["incomplete", "completed"]) {
        let ResponseOutputItem::Message(message) = item else {
            panic!("a message item was expected");
        };
        assert_eq!(message.status, status);
    }
}

#[test]
fn no_choices_completes() {
    let response = assemble(r#"{"id":"c","created":1,"model":"m","choices":[]}"#);
    assert_eq!(response.response.status.as_deref(), Some("completed"));
    assert!(response.response.output.is_empty());
    assert_eq!(response.response.incomplete_details, None);
}

#[test]
fn output_order_places_reasoning_then_messages_then_tools() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[
            {"finish_reason":"tool_calls","message":{"role":"assistant","content":"answer","reasoning_content":"why",
                "tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]}},
            {"finish_reason":"stop","message":{"role":"assistant","content":"second"}}]}"#,
    );
    let output = &response.response.output;
    assert!(matches!(
        output.first(),
        Some(ResponseOutputItem::Message(message)) if message.r#type == "reasoning"
    ));
    assert!(matches!(
        output.get(1),
        Some(ResponseOutputItem::Message(message)) if message.r#type == "message"
    ));
    assert!(matches!(
        output.get(2),
        Some(ResponseOutputItem::Message(message)) if message.r#type == "message"
    ));
    assert!(matches!(output.get(3), Some(ResponseOutputItem::Sdk(_))));
}

#[test]
fn images_suppress_message_content() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi",
            "images":[{"index":0,"type":"image_url","image_url":{"url":"data:image/png;base64,AAA"}}]}}]}"#,
    );
    assert_eq!(response.response.output.len(), 1);
    let ResponseOutputItem::ImageGeneration(OutputImageGenerationCall {
        status, result, id, ..
    }) = &response.response.output[0]
    else {
        panic!("an image item was expected");
    };
    assert_eq!(*status, OutputCallStatus::Completed);
    assert_eq!(result.as_deref(), Some("AAA"));
    assert_eq!(id, "ig_1");
}

#[test]
fn images_that_normalize_away_still_suppress_content() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi",
            "images":[{"index":0,"type":"image_url","image_url":{"url":""}}]}}]}"#,
    );
    assert!(response.response.output.is_empty());
}

#[test]
fn empty_content_makes_a_message() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":""}}]}"#,
    );
    let items = message_items(&response);
    assert_eq!(items.len(), 1);
    let ResponseOutputItem::Message(message) = items[0] else {
        panic!("a message was expected");
    };
    assert_eq!(message.id, "msg_1");
    assert_eq!(message.role, "assistant");
    assert_eq!(message.content.len(), 1);
    assert_eq!(message.content[0].text.as_deref(), Some(""));
    assert_eq!(message.content[0].annotations, Some(Vec::new()));
}

#[test]
fn null_content_makes_no_message() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":null}}]}"#,
    );
    assert!(response.response.output.is_empty());
}

#[test]
fn message_citations_come_from_annotations() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi",
            "annotations":[{"type":"url_citation","url_citation":{"start_index":1,"end_index":2,"url":"https://a","title":"t"}},
                           {"type":"other"}]}}]}"#,
    );
    let ResponseOutputItem::Message(message) = &response.response.output[0] else {
        panic!("a message was expected");
    };
    let annotations = message.content[0].annotations.as_ref().unwrap();
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0].url.as_deref(), Some("https://a"));
    assert_eq!(annotations[0].start_index, Some(1));
    assert_eq!(annotations[0].title.as_deref(), Some("t"));
}

#[test]
fn reasoning_uses_the_first_choice_with_content() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[
            {"finish_reason":"stop","message":{"role":"assistant","content":"one"}},
            {"finish_reason":"length","message":{"role":"assistant","content":"two","reasoning_content":"why"}}]}"#,
    );
    let ResponseOutputItem::Message(reasoning) = &response.response.output[0] else {
        panic!("a reasoning item was expected");
    };
    assert_eq!(reasoning.r#type, "reasoning");
    assert_eq!(reasoning.id, "rs_1");
    assert_eq!(reasoning.status, "incomplete");
    assert_eq!(reasoning.role, "assistant");
    assert_eq!(reasoning.content[0].text.as_deref(), Some("why"));
    assert_eq!(reasoning.extra.get("encrypted_content"), Some(&Json::Null));
}

#[test]
fn thinking_blocks_with_signature_or_data_are_encoded() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi",
            "thinking_blocks":[
                {"signature":"sig","thinking":"t","type":"thinking"},
                {"type":"thinking","thinking":"unsigned"},
                {"data":"zzz","type":"redacted_thinking"}]}}]}"#,
    );
    let ResponseOutputItem::Message(reasoning) = &response.response.output[0] else {
        panic!("a reasoning item was expected");
    };
    assert_eq!(
        reasoning.extra.get("encrypted_content"),
        Some(&parse(
            r#""[{\"type\":\"thinking\",\"thinking\":\"t\",\"signature\":\"sig\"},{\"type\":\"redacted_thinking\",\"data\":\"zzz\"}]""#
        ))
    );
    assert_eq!(reasoning.content, Vec::new());
}

#[test]
fn thinking_blocks_use_each_variant_declaration_order() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi",
            "thinking_blocks":[
                {"cache_control":{"ttl":1},"signature":"sig","thinking":"t","type":"thinking"},
                {"cache_control":{"ttl":2},"data":"d","type":"redacted_thinking"}]}}]}"#,
    );
    let ResponseOutputItem::Message(reasoning) = &response.response.output[0] else {
        panic!("a reasoning item was expected");
    };
    assert_eq!(
        reasoning.extra.get("encrypted_content"),
        Some(&parse(
            r#""[{\"type\":\"thinking\",\"thinking\":\"t\",\"signature\":\"sig\",\"cache_control\":{\"ttl\":1}},{\"type\":\"redacted_thinking\",\"data\":\"d\",\"cache_control\":{\"ttl\":2}}]""#
        ))
    );
}

#[test]
fn thinking_blocks_keep_declared_keys_first_and_extras_after() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi",
            "thinking_blocks":[{"cache_control":null,"signature":"sig","type":"thinking","zz":1,"aa":2}]}}]}"#,
    );
    let ResponseOutputItem::Message(reasoning) = &response.response.output[0] else {
        panic!("a reasoning item was expected");
    };
    assert_eq!(
        reasoning.extra.get("encrypted_content"),
        Some(&parse(
            r#""[{\"type\":\"thinking\",\"signature\":\"sig\",\"cache_control\":null,\"zz\":1,\"aa\":2}]""#
        ))
    );
}

#[test]
fn text_only_reasoning_drops_the_null_cipher_on_serialization() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":null,"reasoning_content":"why"}}]}"#,
    );
    let body = serialized(&response.response);
    let reasoning = element(field(&body, "output"), 0);
    assert_eq!(
        field(reasoning, "content"),
        &parse("[{\"type\":\"output_text\",\"text\":\"why\",\"annotations\":[]}]")
    );
    assert!(matches!(field(reasoning, "status"), Json::Text(status) if status == "completed"));
}

#[test]
fn unsigned_thinking_blocks_make_no_reasoning_item() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi",
            "thinking_blocks":[{"type":"thinking","thinking":"unsigned"}]}}]}"#,
    );
    assert_eq!(response.response.output.len(), 1);
    assert!(matches!(
        &response.response.output[0],
        ResponseOutputItem::Message(message) if message.r#type == "message"
    ));
}

#[test]
fn interpreter_results_replace_matching_function_calls_in_place() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,
            "tool_calls":[{"id":"c","type":"function","function":{"name":"f","arguments":"{}"}},
                          {"id":"d","type":"function","function":{"name":"g","arguments":"{}"}}],
            "provider_specific_fields":{"code_interpreter_results":[
                {"type":"code_interpreter_call","id":"c","status":"completed","code":"x","container_id":null,"outputs":null},
                {"type":"code_interpreter_call","id":"c","status":"failed","code":"y","container_id":"ct","outputs":[]},
                {"type":"code_interpreter_call","id":"e","status":"completed","code":"z","container_id":null,"outputs":null}]}}}]}"#,
    );
    assert_eq!(response.response.output.len(), 2);
    let ResponseOutputItem::CodeInterpreter(call) = &response.response.output[0] else {
        panic!("a code interpreter item was expected");
    };
    assert_eq!(call.id, "c");
    assert_eq!(call.status, OutputCallStatus::Failed);
    assert_eq!(call.code.as_deref(), Some("y"));
    let ResponseOutputItem::Sdk(output) = &response.response.output[1] else {
        panic!("a function call was expected");
    };
    let SdkOutput::FunctionCall(function_call) = output.as_ref() else {
        panic!("a function call was expected");
    };
    assert_eq!(function_call.call_id, "d");
    assert_eq!(function_call.status, Some(FunctionCallStatus::Completed));
}

#[test]
fn interpreter_results_never_replace_custom_calls() {
    let response = assemble_with(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,
            "tool_calls":[{"id":"c","type":"function","function":{"name":"edit","arguments":"{}"}}],
            "provider_specific_fields":{"code_interpreter_results":[
                {"type":"code_interpreter_call","id":"c","status":"completed","code":"x","container_id":null,"outputs":null}]}}}]}"#,
        r#"{"tools":[{"type":"custom","name":"edit","description":"d"}]}"#,
    );
    assert!(matches!(
        &response.response.output[0],
        ResponseOutputItem::CustomTool(item) if item.call_id == "c"
    ));
}

#[test]
fn empty_or_missing_interpreter_results_are_ignored() {
    let chat: ChatCompletionResponse = serde_json::from_str(
        r#"{"id":"c","created":1,"choices":[
            {"finish_reason":"stop","message":{"role":"assistant","content":null,
                "provider_specific_fields":{"code_interpreter_results":[]}}},
            {"finish_reason":"stop","message":{"role":"assistant","content":null,
                "provider_specific_fields":{"code_interpreter_results":""}}}]}"#,
    )
    .unwrap();
    assert!(interpreter_results(&chat.choices).unwrap().is_empty());
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,
            "tool_calls":[{"id":"c","type":"function","function":{"name":"f","arguments":"{}"}}],
            "provider_specific_fields":{"code_interpreter_results":[]}}}]}"#,
    );
    assert!(matches!(
        &response.response.output[0],
        ResponseOutputItem::Sdk(_)
    ));
}

#[test]
fn malformed_interpreter_results_fail() {
    let error = failure(
        r#"{"id":"c","created":1,"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":null,
            "provider_specific_fields":{"code_interpreter_results":[{"type":"code_interpreter_call"}]}}}]}"#,
    );
    assert!(matches!(
        error,
        ChatOutputError::Json {
            field: "code_interpreter_results",
            ..
        }
    ));
}

#[test]
fn hidden_params_are_copied_and_provider_fields_surface() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi"}}],
            "hidden_params":{"model_id":"mid","provider_specific_fields":{"trace":"abc"}}}"#,
    );
    assert_eq!(
        response.hidden_params.get("model_id"),
        Some(&Json::Text("mid".to_owned()))
    );
    assert_eq!(
        response.response.extra.get("provider_specific_fields"),
        Some(&parse(r#"{"trace":"abc"}"#))
    );
}

#[test]
fn empty_hidden_provider_fields_stay_private() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi"}}],
            "hidden_params":{"provider_specific_fields":{}}}"#,
    );
    assert!(
        !response
            .response
            .extra
            .contains_key("provider_specific_fields")
    );
}

#[rstest]
#[case::absent("{}", ToolChoice::Mode(ChoiceMode::Auto))]
#[case::automatic(r#"{"tool_choice":"auto"}"#, ToolChoice::Mode(ChoiceMode::Auto))]
#[case::none(r#"{"tool_choice":"none"}"#, ToolChoice::Mode(ChoiceMode::None))]
#[case::required(
    r#"{"tool_choice":"required"}"#,
    ToolChoice::Mode(ChoiceMode::Required)
)]
#[case::named(r#"{"tool_choice":{"type":"function","name":"f"}}"#, ToolChoice::Named(NamedChoice { r#type: NamedKind::Function, name: "f".to_owned(), extra: BTreeMap::new() }))]
#[case::null(r#"{"tool_choice":null}"#, ToolChoice::Mode(ChoiceMode::Auto))]
fn tool_choice_is_normalized(#[case] request: &str, #[case] expected: ToolChoice) {
    let response = assemble_with(CHAT_A, request);
    assert_eq!(response.response.tool_choice, Some(expected));
}

#[test]
fn usage_projection_keeps_cost_and_details() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi"}}],
            "usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5,"cost":0.125,
                "prompt_tokens_details":{"cached_tokens":1,"cache_write_tokens":4},
                "completion_tokens_details":{"reasoning_tokens":2}}}"#,
    );
    let usage = response.response.usage.as_ref().unwrap();
    assert_eq!(usage.input_tokens, 3);
    assert_eq!(usage.output_tokens, 2);
    assert_eq!(usage.total_tokens, 5);
    assert_eq!(usage.cost, Some("0.125".parse().unwrap()));
    let input = usage.input_tokens_details.as_ref().unwrap();
    assert_eq!(input.cached_tokens, 1);
    assert_eq!(
        input.extra.get("cache_write_tokens"),
        Some(&Json::Number(serde_json::Number::from(4_i32)))
    );
    let output = usage.output_tokens_details.as_ref().unwrap();
    assert_eq!(output.reasoning_tokens, Some(2));
}

#[test]
fn custom_chat_tool_calls_cache_an_empty_function() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":null,
            "tool_calls":[{"id":"x","type":"custom","custom":{"name":"edit","input":"patch"}}]}}]}"#,
    );
    assert!(response.response.output.is_empty());
    assert_eq!(response.cache_writes.len(), 1);
    assert_eq!(response.cache_writes[0].key, "x");
    assert_eq!(response.cache_writes[0].value.function.name, "");
    assert_eq!(
        response.cache_writes[0].value.function.arguments,
        Json::Null
    );
}

#[test]
fn generated_ids_are_prefixed_in_extraction_order() {
    let response = assemble(
        r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"hi","reasoning_content":"why"}}]}"#,
    );
    let ResponseOutputItem::Message(reasoning) = &response.response.output[0] else {
        panic!("a reasoning item was expected");
    };
    let ResponseOutputItem::Message(message) = &response.response.output[1] else {
        panic!("a message item was expected");
    };
    assert_eq!(reasoning.id, "rs_1");
    assert_eq!(message.id, "msg_2");
}
