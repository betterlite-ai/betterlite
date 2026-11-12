#![cfg(test)]

use rstest::rstest;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::config::GenerationConfig;
use super::content::{Content, ToolResultValue};
use super::delta::ContentDeltaValue;
use super::enums::{MediaResolution, ResponseModality, ThinkingLevel, ThinkingSummaries};
use super::events::InteractionSseEvent;
use super::interaction::{AgentConfig, Interaction, InteractionInput};
use super::json::{Json, Object};
use super::response::{
    CancelInteractionResult, DeleteInteractionResult, InteractionsApiResponse,
    InteractionsStreamChunk,
};
use super::status::{InteractionStatus, POLLABLE_STATUSES, TERMINAL_STATUSES};
use super::tools::{Tool, ToolChoice, ToolChoiceType};
use super::usage::Usage;

fn parse<T>(text: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).expect("the fixture deserializes")
}

fn wire_tag<T>(value: &T, field: &str) -> String
where
    T: Serialize,
{
    let encoded = serde_json::to_string(value).expect("the value serializes");
    let object: Object = serde_json::from_str(&encoded).expect("the value is a JSON object");
    match object.get(field) {
        Some(Json::Text(tag)) => tag.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => {
            panic!("the value carries no {field} tag")
        }
    }
}

fn content_wire_type(content: &Content) -> String {
    wire_tag(content, "type")
}

fn tool_wire_type(tool: &Tool) -> String {
    wire_tag(tool, "type")
}

fn delta_wire_type(delta: &ContentDeltaValue) -> String {
    wire_tag(delta, "type")
}

fn event_type(event: &InteractionSseEvent) -> String {
    wire_tag(event, "event_type")
}

fn input_shape(input: &InteractionInput) -> &'static str {
    match input {
        InteractionInput::Text(_) => "text",
        InteractionInput::Content(_) => "content",
        InteractionInput::Turn(_) => "turn",
        InteractionInput::ContentList(_) => "content_list",
        InteractionInput::TurnList(_) => "turn_list",
    }
}

#[test]
fn the_response_reads_every_declared_field() {
    let response: InteractionsApiResponse = parse(
        r#"{"id":"i1","object":"interaction","model":"gemini-2.5-flash","agent":"researcher",
            "status":"completed","created":"2026-01-01T00:00:00Z","updated":"2026-01-01T00:00:01Z",
            "outputs":[{"type":"text","text":"hello"}],"steps":[{"type":"model_output"}],
            "usage":{"total_tokens":3}}"#,
    );
    assert_eq!(response.id.as_deref(), Some("i1"));
    assert_eq!(response.object.as_deref(), Some("interaction"));
    assert_eq!(response.model.as_deref(), Some("gemini-2.5-flash"));
    assert_eq!(response.agent.as_deref(), Some("researcher"));
    assert_eq!(response.status.as_deref(), Some("completed"));
    assert_eq!(response.created.as_deref(), Some("2026-01-01T00:00:00Z"));
    assert_eq!(response.updated.as_deref(), Some("2026-01-01T00:00:01Z"));
    assert_eq!(response.outputs.as_ref().map(Vec::len), Some(1));
    assert_eq!(response.steps.as_ref().map(Vec::len), Some(1));
    assert!(matches!(
        response
            .usage
            .as_ref()
            .and_then(|usage| usage.get("total_tokens")),
        Some(Json::Number(_))
    ));
}

#[test]
fn the_response_defaults_its_object_to_interaction() {
    let response: InteractionsApiResponse = parse(r#"{"id":"i1"}"#);
    assert_eq!(response.object.as_deref(), Some("interaction"));
}

#[test]
fn the_response_keeps_an_explicit_null_object() {
    let response: InteractionsApiResponse = parse(r#"{"id":"i1","object":null}"#);
    assert_eq!(response.object, None);
}

#[test]
fn the_response_keeps_unknown_top_level_fields() {
    let response: InteractionsApiResponse =
        parse(r#"{"id":"i1","vendor_extension":{"kept":true},"vendor_count":2}"#);
    assert!(matches!(
        response.extra.get("vendor_extension"),
        Some(Json::Object(_))
    ));
    assert!(matches!(
        response.extra.get("vendor_count"),
        Some(Json::Number(_))
    ));
    let encoded = serde_json::to_string(&response).expect("the response serializes");
    assert!(encoded.contains("vendor_extension"));
    assert!(encoded.contains("vendor_count"));
    let decoded: InteractionsApiResponse = parse(&encoded);
    assert!(decoded.extra.contains_key("vendor_extension"));
}

#[test]
fn hidden_params_stay_off_the_wire() {
    let mut response: InteractionsApiResponse = parse(r#"{"id":"i1"}"#);
    response
        .hidden_params
        .insert("response_cost".to_owned(), Json::Text("1.5".to_owned()));
    let encoded = serde_json::to_string(&response).expect("the response serializes");
    assert!(!encoded.contains("_hidden_params"));
    assert!(!encoded.contains("response_cost"));
}

#[test]
fn the_stream_chunk_reads_the_streaming_fields() {
    let chunk: InteractionsStreamChunk = parse(
        r#"{"event_type":"content.delta","id":"i1","object":"interaction","model":"gemini",
            "agent":"researcher","status":"in_progress","created":"now","updated":"later",
            "outputs":[{"type":"text"}],"steps":[{"type":"model_output"}],
            "usage":{"total_tokens":1},"delta":{"type":"text","text":"hi"},"index":2,
            "step":{"type":"model_output"},"interaction_id":"i1","interaction":{"id":"i1"},
            "custom":{"n":1}}"#,
    );
    assert_eq!(chunk.event_type.as_deref(), Some("content.delta"));
    assert_eq!(chunk.id.as_deref(), Some("i1"));
    assert_eq!(chunk.object.as_deref(), Some("interaction"));
    assert_eq!(chunk.status.as_deref(), Some("in_progress"));
    assert_eq!(chunk.index, Some(2));
    assert!(chunk.delta.is_some());
    assert!(chunk.step.is_some());
    assert_eq!(chunk.interaction_id.as_deref(), Some("i1"));
    assert!(chunk.interaction.is_some());
    assert!(chunk.extra.contains_key("custom"));
}

#[test]
fn delete_results_default_to_success_and_keep_an_explicit_false() {
    let absent: DeleteInteractionResult = parse(r#"{"id":"i1"}"#);
    assert!(absent.success);
    let refused: DeleteInteractionResult = parse(r#"{"success":false,"id":"i1"}"#);
    assert!(!refused.success);
}

#[test]
fn cancel_results_read_the_id_status_and_unknown_fields() {
    let cancelled: CancelInteractionResult =
        parse(r#"{"id":"i1","status":"cancelled","extra_key":true}"#);
    assert_eq!(cancelled.id.as_deref(), Some("i1"));
    assert_eq!(cancelled.status.as_deref(), Some("cancelled"));
    assert!(cancelled.extra.contains_key("extra_key"));
}

#[rstest]
#[case::in_progress("in_progress", InteractionStatus::InProgress)]
#[case::requires_action("requires_action", InteractionStatus::RequiresAction)]
#[case::completed("completed", InteractionStatus::Completed)]
#[case::failed("failed", InteractionStatus::Failed)]
#[case::cancelled("cancelled", InteractionStatus::Cancelled)]
#[case::incomplete("incomplete", InteractionStatus::Incomplete)]
#[case::budget_exceeded("budget_exceeded", InteractionStatus::BudgetExceeded)]
#[case::queued("queued", InteractionStatus::Queued)]
fn every_status_parses_and_serializes_to_its_wire_name(
    #[case] wire: &str,
    #[case] status: InteractionStatus,
) {
    assert_eq!(InteractionStatus::parse(wire), Some(status));
    let quoted = format!("\"{wire}\"");
    let decoded: InteractionStatus = parse(&quoted);
    assert_eq!(decoded, status);
    assert_eq!(
        serde_json::to_string(&status).expect("the status serializes"),
        quoted
    );
}

#[test]
fn an_unknown_status_does_not_parse() {
    assert_eq!(InteractionStatus::parse("teleported"), None);
    assert_eq!(InteractionStatus::parse(""), None);
}

#[rstest]
#[case::completed(InteractionStatus::Completed, true, false)]
#[case::failed(InteractionStatus::Failed, true, false)]
#[case::cancelled(InteractionStatus::Cancelled, true, false)]
#[case::incomplete(InteractionStatus::Incomplete, true, false)]
#[case::budget_exceeded(InteractionStatus::BudgetExceeded, true, false)]
#[case::requires_action(InteractionStatus::RequiresAction, true, false)]
#[case::in_progress(InteractionStatus::InProgress, false, true)]
#[case::queued(InteractionStatus::Queued, false, true)]
fn the_status_sets_hold_the_declared_statuses(
    #[case] status: InteractionStatus,
    #[case] terminal: bool,
    #[case] pollable: bool,
) {
    assert_eq!(TERMINAL_STATUSES.contains(&status), terminal);
    assert_eq!(POLLABLE_STATUSES.contains(&status), pollable);
    assert_eq!(
        status.produced_output(),
        matches!(
            status,
            InteractionStatus::Completed | InteractionStatus::RequiresAction
        )
    );
}

#[test]
fn an_interaction_record_rejects_an_unknown_status() {
    let error = serde_json::from_str::<Interaction>(r#"{"id":"i1","status":"teleported"}"#)
        .expect_err("an unknown status is rejected");
    assert!(error.to_string().contains("unknown variant"));
}

#[test]
fn an_interaction_record_reads_status_content_and_tools() {
    let record: Interaction = parse(
        r#"{"id":"i1","status":"requires_action","model":"gemini-2.5-flash",
            "background":false,"previous_interaction_id":"i0","system_instruction":"be brief",
            "response_modalities":["text","image"],"response_mime_type":"application/json",
            "response_format":{"type":"object"},
            "outputs":[{"type":"text","text":"hello"},
                       {"type":"function_call","name":"f","arguments":{"a":1},"id":"c1"}],
            "tools":[{"type":"google_search"},{"type":"function","name":"f"}],
            "usage":{"total_input_tokens":3,"total_output_tokens":4}}"#,
    );
    assert_eq!(record.status, InteractionStatus::RequiresAction);
    assert_eq!(record.background, Some(false));
    assert_eq!(record.previous_interaction_id.as_deref(), Some("i0"));
    assert_eq!(record.system_instruction.as_deref(), Some("be brief"));
    assert_eq!(
        record.response_modalities,
        Some(vec![ResponseModality::Text, ResponseModality::Image])
    );
    assert_eq!(
        record.response_mime_type.as_deref(),
        Some("application/json")
    );
    assert!(record.response_format.is_some());
    assert_eq!(record.outputs.as_ref().map(Vec::len), Some(2));
    assert!(matches!(
        record.outputs.as_deref(),
        Some([Content::Text(_), Content::FunctionCall(_)])
    ));
    assert!(matches!(
        record.tools.as_deref(),
        Some([Tool::GoogleSearch, Tool::Function(_)])
    ));
    assert_eq!(
        record
            .usage
            .as_ref()
            .and_then(|usage| usage.total_output_tokens),
        Some(4)
    );
    assert_eq!(record.object, "interaction");
}

#[test]
fn an_interaction_record_reads_configuration_and_input() {
    let record: Interaction = parse(
        r#"{"id":"i1","status":"in_progress",
            "agent_config":{"type":"deep-research","thinking_summaries":"none"},
            "generation_config":{"temperature":0.2,"max_output_tokens":16},
            "input":[{"role":"user","content":"hi"}]}"#,
    );
    assert!(matches!(
        record.agent_config,
        Some(AgentConfig::DeepResearch(_))
    ));
    assert_eq!(
        record
            .generation_config
            .as_ref()
            .and_then(|config| config.max_output_tokens),
        Some(16)
    );
    assert!(matches!(record.input, Some(InteractionInput::TurnList(_))));
}

#[test]
fn an_agent_config_reads_a_deep_research_thinking_setting() {
    let record: Interaction = parse(
        r#"{"id":"i1","status":"in_progress",
            "agent_config":{"type":"deep-research","thinking_summaries":"auto"}}"#,
    );
    match record.agent_config {
        Some(AgentConfig::DeepResearch(config)) => {
            assert_eq!(config.thinking_summaries, Some(ThinkingSummaries::Auto));
        }
        Some(AgentConfig::Dynamic) | None => panic!("the fixture is a deep research config"),
    }
}

#[rstest]
#[case::dynamic(r#"{"type":"dynamic"}"#, false)]
#[case::deep_research(r#"{"type":"deep-research"}"#, true)]
fn agent_configs_read_every_declared_type(#[case] fixture: &str, #[case] deep_research: bool) {
    let record: Interaction = parse(&format!(
        r#"{{"id":"i1","status":"in_progress","agent_config":{fixture}}}"#
    ));
    assert_eq!(
        matches!(record.agent_config, Some(AgentConfig::DeepResearch(_))),
        deep_research
    );
}

#[rstest]
#[case::text(r#""hello""#, "text")]
#[case::content(r#"{"type":"text","text":"hello"}"#, "content")]
#[case::turn(r#"{"role":"user","content":"hello"}"#, "turn")]
#[case::content_list(
    r#"[{"type":"text","text":"a"},{"type":"image","uri":"u"}]"#,
    "content_list"
)]
#[case::turn_list(
    r#"[{"role":"user","content":"a"},{"role":"model","content":[{"type":"text","text":"b"}]}]"#,
    "turn_list"
)]
fn interaction_input_reads_every_declared_shape(#[case] fixture: &str, #[case] shape: &str) {
    let input: InteractionInput = parse(fixture);
    assert_eq!(input_shape(&input), shape);
}

#[rstest]
#[case::text("text", r#"{"type":"text","text":"hello"}"#)]
#[case::image(
    "image",
    r#"{"type":"image","uri":"gs://bucket/x.png","resolution":"high"}"#
)]
#[case::audio("audio", r#"{"type":"audio","data":"AA==","mime_type":"audio/wav"}"#)]
#[case::document("document", r#"{"type":"document","uri":"file:///x.pdf"}"#)]
#[case::video("video", r#"{"type":"video","uri":"gs://v.mp4"}"#)]
#[case::thought(
    "thought",
    r#"{"type":"thought","signature":"sig","summary":[{"type":"text","text":"thinking"}]}"#
)]
#[case::function_call(
    "function_call",
    r#"{"type":"function_call","name":"lookup","arguments":{"q":"x"},"id":"call-1"}"#
)]
#[case::function_result(
    "function_result",
    r#"{"type":"function_result","result":{"items":["a"]},"call_id":"call-1"}"#
)]
#[case::code_execution_call(
    "code_execution_call",
    r#"{"type":"code_execution_call","arguments":{"language":"python","code":"1+1"},"id":"c"}"#
)]
#[case::code_execution_result(
    "code_execution_result",
    r#"{"type":"code_execution_result","result":"2","is_error":false}"#
)]
#[case::url_context_call(
    "url_context_call",
    r#"{"type":"url_context_call","arguments":{"urls":["https://example.com"]}}"#
)]
#[case::url_context_result(
    "url_context_result",
    r#"{"type":"url_context_result","result":[{"url":"https://example.com","status":"success"}]}"#
)]
#[case::google_search_call(
    "google_search_call",
    r#"{"type":"google_search_call","arguments":{"queries":["rust"]}}"#
)]
#[case::google_search_result(
    "google_search_result",
    r#"{"type":"google_search_result","result":[{"url":"https://example.com","title":"Example"}]}"#
)]
#[case::mcp_server_tool_call(
    "mcp_server_tool_call",
    r#"{"type":"mcp_server_tool_call","name":"n","server_name":"s","arguments":{},"id":"c"}"#
)]
#[case::mcp_server_tool_result(
    "mcp_server_tool_result",
    r#"{"type":"mcp_server_tool_result","result":"ok","call_id":"c"}"#
)]
#[case::file_search_result(
    "file_search_result",
    r#"{"type":"file_search_result","result":[{"title":"doc","text":"body"}]}"#
)]
fn the_content_union_reads_every_wire_type(#[case] wire: &str, #[case] fixture: &str) {
    let content: Content = parse(fixture);
    assert_eq!(content_wire_type(&content), wire);
}

#[test]
fn content_keeps_the_declared_required_fields() {
    let missing_id =
        serde_json::from_str::<Content>(r#"{"type":"function_call","name":"f","arguments":{}}"#)
            .expect_err("a function call without an id is rejected");
    assert!(missing_id.to_string().contains("id"));
    let missing_result =
        serde_json::from_str::<Content>(r#"{"type":"function_result","call_id":"c"}"#)
            .expect_err("a function result without a result is rejected");
    assert!(missing_result.to_string().contains("result"));
}

#[test]
fn a_function_result_reads_a_string_or_an_object_payload() {
    let text: Content = parse(r#"{"type":"function_result","result":"ok","call_id":"c"}"#);
    let Content::FunctionResult(text_value) = text else {
        panic!("the fixture is a function result");
    };
    assert!(matches!(text_value.result, ToolResultValue::Text(_)));
    let object: Content =
        parse(r#"{"type":"function_result","result":{"items":["a"]},"call_id":"c"}"#);
    let Content::FunctionResult(object_value) = object else {
        panic!("the fixture is a function result");
    };
    assert!(matches!(object_value.result, ToolResultValue::Object(_)));
}

#[test]
fn an_image_content_reads_its_media_resolution() {
    let content: Content = parse(r#"{"type":"image","uri":"gs://x","resolution":"medium"}"#);
    let Content::Image(image) = content else {
        panic!("the fixture is an image");
    };
    assert_eq!(image.resolution, Some(MediaResolution::Medium));
}

#[rstest]
#[case::function(
    "function",
    r#"{"type":"function","name":"lookup","description":"finds","parameters":{"type":"object"}}"#
)]
#[case::google_search("google_search", r#"{"type":"google_search"}"#)]
#[case::code_execution("code_execution", r#"{"type":"code_execution"}"#)]
#[case::url_context("url_context", r#"{"type":"url_context"}"#)]
#[case::computer_use(
    "computer_use",
    r#"{"type":"computer_use","environment":"browser","excludedPredefinedFunctions":["open"]}"#
)]
#[case::mcp_server(
    "mcp_server",
    r#"{"type":"mcp_server","name":"srv","url":"https://mcp.example",
        "headers":{"x-key":"v"},"allowed_tools":[{"mode":"auto","tools":["search"]}]}"#
)]
#[case::file_search(
    "file_search",
    r#"{"type":"file_search","file_search_store_names":["store"],"top_k":3,"metadata_filter":"a=b"}"#
)]
fn the_tool_union_reads_every_wire_type(#[case] wire: &str, #[case] fixture: &str) {
    let tool: Tool = parse(fixture);
    assert_eq!(tool_wire_type(&tool), wire);
}

#[test]
fn computer_use_reads_the_camel_case_excluded_functions() {
    let tool: Tool = parse(
        r#"{"type":"computer_use","environment":"browser","excludedPredefinedFunctions":["open"]}"#,
    );
    let Tool::ComputerUse(computer_use) = tool else {
        panic!("the fixture is computer use");
    };
    assert_eq!(
        computer_use.excluded_predefined_functions,
        Some(vec!["open".to_owned()])
    );
}

#[test]
fn usage_reads_every_modality_breakdown() {
    let usage: Usage = parse(
        r#"{"total_input_tokens":10,"input_tokens_by_modality":[{"modality":"text","tokens":10}],
            "total_cached_tokens":2,"cached_tokens_by_modality":[{"modality":"image","tokens":2}],
            "total_output_tokens":5,"output_tokens_by_modality":[{"modality":"audio","tokens":5}],
            "total_tool_use_tokens":1,"tool_use_tokens_by_modality":[{"modality":"text","tokens":1}],
            "total_reasoning_tokens":4,"total_tokens":20}"#,
    );
    assert_eq!(usage.total_input_tokens, Some(10));
    assert_eq!(
        usage.input_tokens_by_modality.as_ref().map(Vec::len),
        Some(1)
    );
    assert_eq!(usage.total_cached_tokens, Some(2));
    assert_eq!(
        usage.cached_tokens_by_modality.as_ref().map(Vec::len),
        Some(1)
    );
    assert_eq!(usage.total_output_tokens, Some(5));
    assert_eq!(
        usage.output_tokens_by_modality.as_ref().map(Vec::len),
        Some(1)
    );
    assert_eq!(usage.total_tool_use_tokens, Some(1));
    assert_eq!(
        usage.tool_use_tokens_by_modality.as_ref().map(Vec::len),
        Some(1)
    );
    assert_eq!(usage.total_reasoning_tokens, Some(4));
    assert_eq!(usage.total_tokens, Some(20));
    assert_eq!(
        usage
            .output_tokens_by_modality
            .as_ref()
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.modality),
        Some(ResponseModality::Audio)
    );
}

#[test]
fn generation_config_reads_sampling_and_tool_choice() {
    let config: GenerationConfig = parse(
        r#"{"temperature":0.2,"top_p":0.9,"seed":7,"stop_sequences":["END"],
            "tool_choice":"auto","thinking_level":"high","thinking_summaries":"auto",
            "max_output_tokens":32,"speech_config":[{"voice":"Kore","language":"en","speaker":"s"}]}"#,
    );
    assert!(config.temperature.is_some());
    assert!(config.top_p.is_some());
    assert_eq!(config.seed, Some(7));
    assert_eq!(config.stop_sequences, Some(vec!["END".to_owned()]));
    assert!(matches!(
        config.tool_choice,
        Some(ToolChoice::Mode(ToolChoiceType::Auto))
    ));
    assert_eq!(config.thinking_level, Some(ThinkingLevel::High));
    assert_eq!(config.thinking_summaries, Some(ThinkingSummaries::Auto));
    assert_eq!(config.max_output_tokens, Some(32));
    assert_eq!(config.speech_config.as_ref().map(Vec::len), Some(1));
}

#[test]
fn a_tool_choice_config_reads_allowed_tools() {
    let config: GenerationConfig =
        parse(r#"{"tool_choice":{"allowed_tools":{"mode":"validated","tools":["search"]}}}"#);
    match config.tool_choice {
        Some(ToolChoice::Config(choice)) => {
            assert!(choice.allowed_tools.is_some());
        }
        Some(ToolChoice::Mode(_)) | None => panic!("the fixture is a tool choice config"),
    }
}

#[rstest]
#[case::step_start("step.start", r#"{"event_type":"step.start","index":0}"#)]
#[case::step_delta("step.delta", r#"{"event_type":"step.delta","delta":{"text":"x"}}"#)]
#[case::step_stop("step.stop", r#"{"event_type":"step.stop","index":1,"status":"done"}"#)]
#[case::interaction_created(
    "interaction.created",
    r#"{"event_type":"interaction.created","interaction":{"id":"i1"}}"#
)]
#[case::interaction_in_progress(
    "interaction.in_progress",
    r#"{"event_type":"interaction.in_progress","interaction_id":"i1"}"#
)]
#[case::interaction_completed(
    "interaction.completed",
    r#"{"event_type":"interaction.completed","interaction":{"id":"i1","status":"completed"}}"#
)]
#[case::interaction_requires_action(
    "interaction.requires_action",
    r#"{"event_type":"interaction.requires_action","interaction_id":"i1"}"#
)]
#[case::interaction_start(
    "interaction.start",
    r#"{"event_type":"interaction.start","interaction":{"id":"i1","status":"in_progress"}}"#
)]
#[case::interaction_complete(
    "interaction.complete",
    r#"{"event_type":"interaction.complete","interaction":{"id":"i1","status":"completed"}}"#
)]
#[case::interaction_status_update(
    "interaction.status_update",
    r#"{"event_type":"interaction.status_update","status":"queued"}"#
)]
#[case::content_start(
    "content.start",
    r#"{"event_type":"content.start","content":{"type":"text","text":"x"}}"#
)]
#[case::content_delta(
    "content.delta",
    r#"{"event_type":"content.delta","delta":{"type":"text","text":"x"}}"#
)]
#[case::content_stop("content.stop", r#"{"event_type":"content.stop","index":0}"#)]
#[case::error(
    "error",
    r#"{"event_type":"error","error":{"code":"bad","message":"no"}}"#
)]
fn the_sse_event_union_reads_every_event_type(#[case] wire: &str, #[case] fixture: &str) {
    let event: InteractionSseEvent = parse(fixture);
    assert_eq!(event_type(&event), wire);
}

#[test]
fn an_sse_event_rejects_an_unknown_or_missing_event_type() {
    let unknown =
        serde_json::from_str::<InteractionSseEvent>(r#"{"event_type":"content.replaced"}"#)
            .expect_err("an unknown event type is rejected");
    assert!(unknown.to_string().contains("unknown variant"));
    let missing = serde_json::from_str::<InteractionSseEvent>(r#"{"index":0}"#)
        .expect_err("an event without an event type is rejected");
    assert!(missing.to_string().contains("event_type"));
}

#[rstest]
#[case::text("text", r#"{"type":"text","text":"x"}"#)]
#[case::image("image", r#"{"type":"image"}"#)]
#[case::audio("audio", r#"{"type":"audio"}"#)]
#[case::document("document", r#"{"type":"document"}"#)]
#[case::video("video", r#"{"type":"video"}"#)]
#[case::thought_summary(
    "thought_summary",
    r#"{"type":"thought_summary","content":{"type":"text"}}"#
)]
#[case::thought_signature("thought_signature", r#"{"type":"thought_signature","signature":"s"}"#)]
#[case::function_call(
    "function_call",
    r#"{"type":"function_call","name":"f","arguments":{}}"#
)]
#[case::function_result("function_result", r#"{"type":"function_result","result":"ok"}"#)]
#[case::code_execution_call("code_execution_call", r#"{"type":"code_execution_call","id":"c"}"#)]
#[case::code_execution_result(
    "code_execution_result",
    r#"{"type":"code_execution_result","result":"1"}"#
)]
#[case::url_context_call(
    "url_context_call",
    r#"{"type":"url_context_call","arguments":{"urls":["u"]}}"#
)]
#[case::url_context_result(
    "url_context_result",
    r#"{"type":"url_context_result","result":[{"url":"u","status":"success"}]}"#
)]
#[case::google_search_call(
    "google_search_call",
    r#"{"type":"google_search_call","arguments":{"queries":["q"]}}"#
)]
#[case::google_search_result(
    "google_search_result",
    r#"{"type":"google_search_result","result":[{"url":"u"}]}"#
)]
#[case::mcp_server_tool_call(
    "mcp_server_tool_call",
    r#"{"type":"mcp_server_tool_call","name":"n","server_name":"s","arguments":{}}"#
)]
#[case::mcp_server_tool_result(
    "mcp_server_tool_result",
    r#"{"type":"mcp_server_tool_result","result":"ok"}"#
)]
#[case::file_search_result(
    "file_search_result",
    r#"{"type":"file_search_result","result":[{"title":"t"}]}"#
)]
fn the_content_delta_union_reads_every_wire_type(#[case] wire: &str, #[case] fixture: &str) {
    let delta: ContentDeltaValue = parse(fixture);
    assert_eq!(delta_wire_type(&delta), wire);
}
