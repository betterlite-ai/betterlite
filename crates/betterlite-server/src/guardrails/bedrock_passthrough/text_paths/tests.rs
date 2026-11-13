#![cfg(test)]

use std::io;
use std::str;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use super::steps::{PathStep, TextPath};
use super::{SkipFlags, collect_output_texts, collect_request_texts, write_back_texts};

fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).expect("valid JSON fixture")
}

fn key(name: &str) -> PathStep {
    PathStep::Key(name.to_owned())
}

fn text_path(steps: Vec<PathStep>) -> TextPath {
    TextPath(steps)
}

fn texts(collected: &[(TextPath, String)]) -> Vec<&str> {
    collected.iter().map(|(_, text)| text.as_str()).collect()
}

const REQUEST: &str = r#"{
  "system": [{"text": "be nice"}, {"text": ""}, "skip"],
  "messages": [
    {"role": "user", "content": [
      {"text": "hello"},
      {"toolUse": {"input": {"a": "inner", "list": ["one", {"d": "two"}], "nested": {"b": "deep", "c": "deeper"}}}},
      {"toolResult": {"content": [{"text": "result text", "json": {"j": "json text"}}, "skip"]}},
      {"text": ""}
    ]},
    "skip",
    {"role": "assistant", "content": "not a list"}
  ],
  "additionalModelRequestFields": {"deep": {"x": "nested"}, "top": "additional"},
  "toolConfig": {"tools": [{"toolSpec": {"name": "never scanned"}}]}
}"#;

#[test]
fn the_request_collector_gathers_every_place_in_litellms_order() {
    let body = value(REQUEST);
    let collected = collect_request_texts(&body, SkipFlags::default());
    assert_eq!(
        texts(&collected),
        vec![
            "be nice",
            "hello",
            "inner",
            "deep",
            "deeper",
            "one",
            "two",
            "result text",
            "json text",
            "additional",
            "nested",
        ]
    );
}

#[test]
fn the_request_collector_records_the_route_to_each_text() {
    let body = value(REQUEST);
    let collected = collect_request_texts(&body, SkipFlags::default());
    let paths: Vec<&TextPath> = collected.iter().map(|(path, _)| path).collect();
    assert_eq!(
        paths[0],
        &text_path(vec![key("system"), PathStep::Index(0), key("text")])
    );
    let first_block = [
        key("messages"),
        PathStep::Index(0),
        key("content"),
        PathStep::Index(0),
        key("text"),
    ];
    assert_eq!(paths[1], &text_path(first_block.to_vec()));
}

#[test]
fn the_request_collector_records_nested_tool_and_additional_routes() {
    let body = value(REQUEST);
    let collected = collect_request_texts(&body, SkipFlags::default());
    let paths: Vec<&TextPath> = collected.iter().map(|(path, _)| path).collect();
    let mut tool_input = vec![
        key("messages"),
        PathStep::Index(0),
        key("content"),
        PathStep::Index(1),
        key("toolUse"),
        key("input"),
    ];
    tool_input.push(key("a"));
    assert_eq!(paths[2], &text_path(tool_input));
    let json = [
        key("messages"),
        PathStep::Index(0),
        key("content"),
        PathStep::Index(2),
        key("toolResult"),
        key("content"),
        PathStep::Index(0),
        key("json"),
        key("j"),
    ];
    assert_eq!(paths[8], &text_path(json.to_vec()));
    assert_eq!(
        paths[10],
        &text_path(vec![
            key("additionalModelRequestFields"),
            key("deep"),
            key("x"),
        ])
    );
}

#[test]
fn skip_system_leaves_the_system_blocks_out() {
    let body = value(REQUEST);
    let collected = collect_request_texts(
        &body,
        SkipFlags {
            system: true,
            tool: false,
        },
    );
    assert!(!texts(&collected).contains(&"be nice"));
    assert_eq!(collected.len(), 10);
}

#[test]
fn skip_tool_drops_whole_tool_blocks_and_nothing_else() {
    let body = value(REQUEST);
    let collected = collect_request_texts(
        &body,
        SkipFlags {
            system: false,
            tool: true,
        },
    );
    assert_eq!(
        texts(&collected),
        vec!["be nice", "hello", "additional", "nested"]
    );
}

#[test]
fn a_text_next_to_a_tool_block_is_still_scanned_without_skip_tool() {
    let body = value(
        r#"{"messages": [{"content": [{"text": "beside", "toolUse": {"input": {"q": "inside"}}}]}]}"#,
    );
    let collected = collect_request_texts(&body, SkipFlags::default());
    assert_eq!(texts(&collected), vec!["beside", "inside"]);
    let skipped = collect_request_texts(
        &body,
        SkipFlags {
            system: false,
            tool: true,
        },
    );
    assert!(skipped.is_empty());
}

#[test]
fn the_deep_walk_visits_direct_strings_then_containers_last_first() {
    // LiteLLM pushes both nested objects in key order and pops them last in, first out
    let body = value(
        r#"{"messages": [{"content": [{"toolUse": {"input": {"first": "direct", "nested": {"inner": "second"}, "other": {"inner": "third"}}}}]}]}"#,
    );
    let collected = collect_request_texts(&body, SkipFlags::default());
    assert_eq!(texts(&collected), vec!["direct", "third", "second"]);
}

#[test]
fn a_null_tool_input_yields_nothing() {
    let body = value(r#"{"messages": [{"content": [{"toolUse": {"input": null}}]}]}"#);
    assert!(collect_request_texts(&body, SkipFlags::default()).is_empty());
}

#[test]
fn a_bare_string_where_an_object_belongs_yields_nothing() {
    let tool_use =
        value(r#"{"messages": [{"content": [{"toolUse": {"input": "plain string"}}]}]}"#);
    assert!(collect_request_texts(&tool_use, SkipFlags::default()).is_empty());
    let additional = value(r#"{"additionalModelRequestFields": "plain string"}"#);
    assert!(collect_request_texts(&additional, SkipFlags::default()).is_empty());
    let json = value(
        r#"{"messages": [{"content": [{"toolResult": {"content": [{"json": "plain string"}]}}]}]}"#,
    );
    assert!(collect_request_texts(&json, SkipFlags::default()).is_empty());
}

#[test]
fn empty_strings_are_never_collected() {
    let body = value(r#"{"system": [{"text": ""}], "messages": [{"content": [{"text": ""}]}]}"#);
    assert!(collect_request_texts(&body, SkipFlags::default()).is_empty());
}

#[test]
fn a_non_object_body_yields_nothing() {
    assert!(collect_request_texts(&value("[]"), SkipFlags::default()).is_empty());
}

const RESPONSE_CONTENT: &str = r#"[
  {"text": "answer", "reasoningContent": {"reasoningText": {"text": "thought", "signature": "sig"}}},
  "skip",
  {"toolUse": {"input": {"q": "tool input", "r": "second"}}},
  {"citationsContent": {"content": [{"text": "cited", "source": "source"}, {}], "citations": [{"source": "never"}]}},
  {"reasoningContent": {"redactedContent": {"text": "never"}}}
]"#;

fn array(json: &str) -> Vec<GuardrailValue> {
    let GuardrailValue::Array(items) = value(json) else {
        panic!("array fixture");
    };
    items
}

#[test]
fn the_output_collector_gathers_text_tool_reasoning_and_citations() {
    let collected = collect_output_texts(&array(RESPONSE_CONTENT));
    assert_eq!(
        texts(&collected),
        vec!["answer", "thought", "tool input", "second", "cited"]
    );
}

#[test]
fn the_output_collector_skips_signatures_and_sources() {
    let collected = collect_output_texts(&array(RESPONSE_CONTENT));
    let collected_texts = texts(&collected);
    assert!(!collected_texts.contains(&"sig"));
    assert!(!collected_texts.contains(&"source"));
    assert!(!collected_texts.contains(&"never"));
}

#[test]
fn the_output_collector_paths_start_at_the_block_index() {
    let collected = collect_output_texts(&array(RESPONSE_CONTENT));
    assert_eq!(
        collected[0].0,
        text_path(vec![PathStep::Index(0), key("text")])
    );
    assert_eq!(
        collected[1].0,
        text_path(vec![
            PathStep::Index(0),
            key("reasoningContent"),
            key("reasoningText"),
            key("text"),
        ])
    );
    assert_eq!(
        collected[4].0,
        text_path(vec![
            PathStep::Index(3),
            key("citationsContent"),
            key("content"),
            PathStep::Index(0),
            key("text"),
        ])
    );
}

#[test]
fn write_back_sets_the_text_at_each_path_in_order() {
    let mut body = value(
        r#"{"system": [{"text": "one"}], "messages": [{"content": [{"text": "two"}, {"toolUse": {"input": {"k": "three"}}}]}]}"#,
    );
    let holders = collect_request_texts(&body, SkipFlags::default());
    let paths: Vec<TextPath> = holders.into_iter().map(|(path, _)| path).collect();
    let returned = ["ONE".to_owned(), "TWO".to_owned(), "THREE".to_owned()];
    write_back_texts(&mut body, &paths, &returned);
    let collected = collect_request_texts(&body, SkipFlags::default());
    assert_eq!(texts(&collected), vec!["ONE", "TWO", "THREE"]);
}

#[test]
fn write_back_keeps_the_unreturned_texts_and_warns() {
    let mut body = value(r#"{"messages": [{"content": [{"text": "one"}, {"text": "two"}]}]}"#);
    let holders = collect_request_texts(&body, SkipFlags::default());
    let paths: Vec<TextPath> = holders.into_iter().map(|(path, _)| path).collect();
    let records = Arc::new(Mutex::new(String::new()));
    let capture = Capture(Arc::clone(&records));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture)
        .with_ansi(false)
        .finish();
    with_default(subscriber, || {
        write_back_texts(&mut body, &paths, &["ONE".to_owned()]);
    });
    let collected = collect_request_texts(&body, SkipFlags::default());
    assert_eq!(texts(&collected), vec!["ONE", "two"]);
    let logged = records.lock().expect("not poisoned").clone();
    assert!(logged.contains(
        "guardrail returned 1 texts for 2 extracted fields; the unreturned fields keep their original text"
    ));
}

#[test]
fn write_back_logs_no_warning_when_every_text_is_returned() {
    let mut body = value(r#"{"messages": [{"content": [{"text": "one"}, {"text": "two"}]}]}"#);
    let holders = collect_request_texts(&body, SkipFlags::default());
    let paths: Vec<TextPath> = holders.into_iter().map(|(path, _)| path).collect();
    let records = Arc::new(Mutex::new(String::new()));
    let capture = Capture(Arc::clone(&records));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture)
        .with_ansi(false)
        .finish();
    with_default(subscriber, || {
        write_back_texts(&mut body, &paths, &["ONE".to_owned(), "TWO".to_owned()]);
    });
    let logged = records.lock().expect("not poisoned").clone();
    assert!(!logged.contains("guardrail returned"));
}

#[test]
fn write_back_drops_returned_texts_past_the_holders() {
    let mut body = value(r#"{"messages": [{"content": [{"text": "one"}]}]}"#);
    let holders = collect_request_texts(&body, SkipFlags::default());
    let paths: Vec<TextPath> = holders.into_iter().map(|(path, _)| path).collect();
    write_back_texts(
        &mut body,
        &paths,
        &["ONE".to_owned(), "EXTRA".to_owned(), "MORE".to_owned()],
    );
    let collected = collect_request_texts(&body, SkipFlags::default());
    assert_eq!(texts(&collected), vec!["ONE"]);
    let serialized = serde_json::to_string(&body).expect("serializes");
    assert!(!serialized.contains("EXTRA"));
}

#[test]
fn write_back_skips_a_path_that_no_longer_reaches_text() {
    let mut body = value(r#"{"messages": [{"content": [{"text": "one"}]}]}"#);
    let holders = collect_request_texts(&body, SkipFlags::default());
    let paths: Vec<TextPath> = holders.into_iter().map(|(path, _)| path).collect();
    body = value(r#"{"messages": []}"#);
    write_back_texts(&mut body, &paths, &["ONE".to_owned()]);
    assert_eq!(
        serde_json::to_string(&body).expect("serializes"),
        r#"{"messages":[]}"#
    );
}

#[derive(Clone)]
struct Capture(Arc<Mutex<String>>);

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = str::from_utf8(buf)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        self.0
            .lock()
            .map_err(|source| io::Error::other(source.to_string()))?
            .push_str(text);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Capture {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}
