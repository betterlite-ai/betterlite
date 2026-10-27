#![cfg(test)]

use betterlite_logging::analytics::{lunary, messages, params, python, wandb, wire};
use rstest::rstest;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;

type Fields = BTreeMap<String, String>;
type TextMessages = messages::Messages<String, String>;
type Extra = params::Extra<BTreeMap<String, bool>, Vec<Tool>, Fields>;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
struct Tool {
    #[serde(rename = "type")]
    kind: String,
    function: ToolDefinition,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
struct ToolDefinition {
    name: String,
}

#[rstest]
#[case::missing("{}", 0, 0)]
#[case::prompt_only(r#"{"prompt_tokens":4}"#, 0, 4)]
#[case::both(r#"{"completion_tokens":2,"prompt_tokens":4}"#, 2, 4)]
fn parses_usage_with_zero_fallback(
    #[case] body: &str,
    #[case] completion: u64,
    #[case] prompt: u64,
) {
    let usage: wire::Usage = serde_json::from_str(body).unwrap();
    assert_eq!(
        wire::Tokens::from(&usage),
        wire::Tokens { completion, prompt }
    );
    assert_eq!(
        serde_json::to_string(&wire::Tokens::from(&usage)).unwrap(),
        format!("{{\"completion\":{completion},\"prompt\":{prompt}}}")
    );
}

#[rstest]
#[case::text(r#""hello""#, r#""hello""#)]
#[case::single(r#"["hello"]"#, r#""hello""#)]
#[case::empty("[]", "[]")]
#[case::many(r#"["one","two"]"#, r#"["one","two"]"#)]
#[case::nested(
    r#"[{"message":{"message":{"role":"assistant","content":"done"}}}]"#,
    r#"{"role":"assistant","content":"done"}"#
)]
#[case::empty_calls(
    r#"{"role":"assistant","content":null,"tool_calls":[]}"#,
    r#"{"role":"assistant","content":null}"#
)]
fn parses_messages(#[case] input: &str, #[case] expected: &str) {
    let messages: TextMessages = serde_json::from_str(input).unwrap();
    assert_eq!(
        serde_json::to_string(&messages::parse_messages(&messages)).unwrap(),
        expected
    );
}

#[test]
fn parses_messages_and_tool_call_variants() {
    let input: TextMessages = serde_json::from_str(
        r#"[{"role":"assistant","content":"done","tool_calls":[
        {"type":"function","id":"one","function":{"name":"search","arguments":"{}"}},
        {"type":"custom","id":"two","custom":{"name":"lookup","input":"query"}},
        {"type":"function","id":"ignored"}]}]"#,
    )
    .unwrap();
    let output = messages::parse_messages(&input);
    assert_eq!(
        serde_json::to_string(&output).unwrap(),
        r#"{"role":"assistant","content":"done","tool_calls":[{"id":"one","type":"function","function":{"name":"search","arguments":"{}"}},{"id":"two","type":"custom","function":{"name":"lookup","arguments":"query"}}]}"#
    );
}

#[test]
fn custom_tool_takes_precedence() {
    let calls: Vec<messages::ToolCall<String>> = serde_json::from_str(r#"[{"id":"both","type":"custom",
        "function":{"name":"wrong","arguments":"wrong"},"custom":{"name":"right","input":"right"}}]"#).unwrap();
    let parsed = messages::parse_tool_calls(&calls);
    assert_eq!(parsed[0].function.name, "right");
    assert_eq!(parsed[0].function.arguments, "right");
}

#[test]
fn tools_list_of_dicts_stays_typed() {
    let extra: Extra = serde_json::from_str(
        r#"{"temperature":1,"extra_headers":{"Template-Id":"old"},"user":"ignored"}"#,
    )
    .unwrap();
    let optional: Extra = serde_json::from_str(r#"{"extra_body":{"discard":"yes"},"extra_headers":{"Template-Id":"new","Other":"ignored"},
        "tools":[{"type":"function","function":{"name":"lookup"}}],"metadata":{"nested":true},"temperature":2}"#).unwrap();
    let (cleaned, template) = params::clean_extra_params(&extra, &optional).unwrap();
    assert_eq!(template.as_deref(), Some("new"));
    assert_eq!(
        serde_json::to_string(&cleaned).unwrap(),
        r#"{"tools":[{"type":"function","function":{"name":"lookup"}}],"metadata":"{'nested': True}","temperature":2}"#
    );
}

#[test]
fn tools_dictionary_is_preserved() {
    let extra: params::Extra<Fields, Tool, Fields> =
        serde_json::from_str(r#"{"tools":{"type":"function","function":{"name":"lookup"}}}"#)
            .unwrap();
    let optional = serde_json::from_str("{}").unwrap();
    let (cleaned, _) = params::clean_extra_params(&extra, &optional).unwrap();
    assert_eq!(cleaned.tools, extra.params.tools);
}

#[test]
fn null_parameter_becomes_python_none() {
    let extra: Extra = serde_json::from_str(r#"{"stop":null}"#).unwrap();
    let optional = serde_json::from_str("{}").unwrap();
    let (cleaned, _) = params::clean_extra_params(&extra, &optional).unwrap();
    assert_eq!(
        serde_json::to_string(&cleaned).unwrap(),
        r#"{"stop":"None"}"#
    );
}

#[test]
fn optional_headers_replace_extra_headers() {
    let extra: Extra = serde_json::from_str(r#"{"extra_headers":{"Template-Id":"old"}}"#).unwrap();
    let optional = serde_json::from_str(r#"{"extra_headers":{"Other":"value"}}"#).unwrap();
    assert_eq!(
        params::clean_extra_params(&extra, &optional).unwrap().1,
        None
    );
}

#[rstest]
#[case::minimum("0.1.43")]
#[case::postrelease("0.1.43.post1")]
#[case::epoch("1!0.0")]
#[case::local("0.1.43+build")]
#[case::future_prerelease("0.1.44rc1")]
#[case::short("1")]
fn accepts_pep440_release_versions(#[case] version: &str) {
    lunary::check_version(version).unwrap();
}

#[rstest]
#[case::old("0.1.42")]
#[case::prerelease("0.1.43-rc1")]
#[case::dev("0.1.43.dev1")]
fn rejects_pep440_prerelease(#[case] version: &str) {
    assert!(matches!(
        lunary::check_version(version),
        Err(lunary::LunaryError::OutdatedVersion { .. })
    ));
}

#[test]
fn version_errors_keep_their_cause() {
    let error = lunary::check_version("invalid").unwrap_err();
    assert!(Error::source(&error).is_some());
}

#[test]
fn emits_paired_start_terminal_events() {
    let request: lunary::Request<String, String, Fields, Vec<Tool>> = serde_json::from_str(
        r#"{
        "event_type":"llm","run_id":"run","model":"gpt","input":["hello"],
        "metadata":{"parent_run_id":"parent","tags":["tag"],"team":"analytics"},
        "params":{"temperature":1},"template_id":"template","user_id":"user",
        "start_time":"2026-10-27T08:00:00.123456+02:00","end_time":"2026-10-27T06:00:01Z"}"#,
    )
    .unwrap();
    let response = serde_json::from_str(r#"{"choices":[{"message":{"role":"assistant","content":"world"}}],"usage":{"prompt_tokens":4}}"#).unwrap();
    let [start, terminal] = lunary::events(&request, Some(&response), "end", None);
    assert_eq!(
        (start.event.as_str(), terminal.event.as_str()),
        ("start", "end")
    );
    assert_eq!(
        (start.run_id.as_str(), terminal.run_id.as_str()),
        ("run", "run")
    );
    assert_eq!(start.timestamp, "2026-10-27T06:00:00.123456+00:00");
    assert_eq!(terminal.timestamp, "2026-10-27T06:00:01+00:00");
    let start_json = serde_json::to_string(&start).unwrap();
    assert!(start_json.contains(r#""input":"hello""#));
    assert!(start_json.contains(r#""team":"analytics""#));
    assert!(start_json.contains(r#""parent_run_id":"parent""#));
    assert!(start_json.contains(r#""tags":["tag"]"#));
    assert!(start_json.contains(r#""params":{"temperature":1}"#));
    assert!(start_json.contains(r#""template_id":"template""#));
    assert_eq!(
        serde_json::to_string(&terminal).unwrap(),
        r#"{"type":"llm","event":"end","run_id":"run","timestamp":"2026-10-27T06:00:01+00:00","runtime":"litellm","error":null,"output":{"role":"assistant","content":"world"},"token_usage":{"completion":0,"prompt":4}}"#
    );
    let [_, failed] = lunary::events(&request, None, "error", Some("failed"));
    assert!(
        serde_json::to_string(&failed)
            .unwrap()
            .contains(r#""error":{"stack":"failed"},"output":null,"token_usage":null"#)
    );
}

#[rstest]
#[case::edit(
    "edit",
    r#"{"instruction":"fix","input":"bad"}"#,
    r#"{"text":"good"}"#,
    "\n\n**Instruction**: fix\n\n**Input**: bad\n",
    "\n\n**Edited**: good\n"
)]
#[case::completion(
    "text_completion",
    r#"{"prompt":"hello"}"#,
    r#"{"text":"world"}"#,
    "\n\n**Prompt**: hello\n",
    "\n\n**Completion**: world\n"
)]
#[case::chat(
    "chat.completion",
    r#"{"messages":[{"role":"user","content":"hello"},{"role":"assistant","content":"hi"}]}"#,
    r#"{"message":{"role":"assistant","content":"world"}}"#,
    "\n\n**user**: hello\n\n\n**assistant**: hi\n",
    "\n\n**assistant**: world\n"
)]
fn resolves_trace_trees(
    #[case] object: &str,
    #[case] request_json: &str,
    #[case] choice: &str,
    #[case] input: &str,
    #[case] output: &str,
) {
    let request: wire::Request<Fields> = serde_json::from_str(request_json).unwrap();
    let response: wire::Response<Fields> = serde_json::from_str(&format!(r#"{{"object":"{object}","model":"gpt","created":10,"choices":[{choice},{choice}],"id":"response"}}"#)).unwrap();
    let trace = wandb::resolve(&request, &response, Decimal::new(25, 2)).unwrap();
    assert_eq!(trace.root_span.name, format!("gpt_{object}_10"));
    assert_eq!(trace.root_span.results.len(), 2);
    assert_eq!(trace.root_span.results[0].inputs["request"], input);
    assert_eq!(trace.root_span.results[0].outputs["response"], output);
    assert_eq!(trace.root_span.results[0], trace.root_span.results[1]);
    assert_eq!(trace.root_span.start_time_ms, 10_000);
    assert_eq!(trace.root_span.end_time_ms, 10_250);
    assert_eq!(trace.root_span.attributes, response);
    assert_eq!(trace.model_dict.request, request);
    assert_eq!(trace.model_dict.response, response);
    assert_eq!(trace.model_dict.kind, "openai");
}

#[rstest]
#[case::round_up("10.0006", "0.0006", 10001, 10002)]
#[case::even_tie("10.0005", "0.0025", 10000, 10002)]
#[case::odd_tie("10.0015", "0.0015", 10002, 10004)]
#[case::negative("-1", "-0.001", -1000, -1001)]
fn rounds_timestamp_milliseconds(
    #[case] created: &str,
    #[case] elapsed: &str,
    #[case] start: i64,
    #[case] end: i64,
) {
    let request: wire::Request<Fields> = serde_json::from_str(r#"{"prompt":"hi"}"#).unwrap();
    let response: wire::Response<Fields> = serde_json::from_str(&format!(
        r#"{{"object":"text_completion","created":{created},"choices":[]}}"#
    ))
    .unwrap();
    let trace = wandb::resolve(&request, &response, elapsed.parse().unwrap()).unwrap();
    assert_eq!(trace.root_span.start_time_ms, start);
    assert_eq!(trace.root_span.end_time_ms, end);
    assert!(trace.root_span.name.starts_with("openai_"));
}

#[rstest]
#[case::unknown("unknown", r#"{"prompt":"hi"}"#, "[]")]
#[case::missing_prompt("text_completion", "{}", "[]")]
#[case::missing_instruction("edit", r#"{"input":"hi"}"#, "[]")]
#[case::missing_input("edit", r#"{"instruction":"fix"}"#, "[]")]
#[case::missing_messages("chat.completion", "{}", "[]")]
#[case::bad_choice("text_completion", r#"{"prompt":"hi"}"#, r#"[{"text":"ok"},{}]"#)]
#[case::bad_chat_choice("chat.completion", r#"{"messages":[]}"#, r#"[{"text":"wrong"}]"#)]
fn resolver_swallows_errors(#[case] object: &str, #[case] input: &str, #[case] choices: &str) {
    let request: wire::Request<Fields> = serde_json::from_str(input).unwrap();
    let response: wire::Response<Fields> = serde_json::from_str(&format!(
        r#"{{"object":"{object}","created":0,"choices":{choices}}}"#
    ))
    .unwrap();
    assert!(wandb::resolve(&request, &response, Decimal::ZERO).is_none());
}

#[test]
fn timestamp_overflow_is_swallowed() {
    let request: wire::Request<Fields> = serde_json::from_str(r#"{"prompt":"hi"}"#).unwrap();
    let mut response: wire::Response<Fields> =
        serde_json::from_str(r#"{"object":"text_completion","created":0,"choices":[]}"#).unwrap();
    assert!(wandb::resolve(&request, &response, Decimal::MAX).is_none());
    response.created = Decimal::MAX;
    assert!(wandb::resolve(&request, &response, Decimal::ZERO).is_none());
}

#[test]
fn structured_parameters_use_python_repr() {
    let values = BTreeMap::from([("flags", vec![Some(true), None, Some(false)])]);
    assert_eq!(
        python::repr(&values).unwrap(),
        "{'flags': [True, None, False]}"
    );
    assert_eq!(
        python::repr(&vec!["don't", "line\n\t\u{0}"]).unwrap(),
        "[\"don't\", 'line\\n\\t\\x00']"
    );
}
