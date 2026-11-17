#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::mem;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::responses_objects::{ResponseApiUsage, ResponseOutputItem};
use betterlite_openai::api::responses_shapes::{
    FunctionCallStatus, GenericResponseOutputItem, OutputFunctionToolCall, OutputText,
};
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResult;
use rust_decimal::Decimal;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry;

use crate::api::responses_tool_namespaces::values::Fields;

use super::error::FileSearchToolsError;
use super::outputs::{
    build_file_search_call_output, build_message_output, citation_annotations, response_output_text,
};
use super::preparation::{
    extract_tool_call_fields, prepare_file_search_call, resolve_queries_from_args,
};
use super::results::{
    NO_DEFAULT, NO_RESULTS, field, format_results_as_tool_output, joined_content_text,
    search_results_for_include,
};
use super::synthesis::{
    OriginalModel, OriginalResponse, OutputItems, SynthesizedResponse, synthesize_response,
};
use super::tools::{build_function_tool, replace_file_search_tools};
use super::wire;

const TOOL_JSON: &str = r#"{"type":"function","name":"litellm_file_search","description":"Search the knowledge base for information relevant to the query. Use this whenever you need to look up specific facts, documents, or content from the vector store. You can provide multiple queries to search for different aspects of the information.","parameters":{"type":"object","properties":{"queries":{"type":"array","items":{"type":"string"},"description":"One or more search queries to look up in the vector store. Multiple queries help find comprehensive information from different angles."},"vector_store_id":{"type":"string","description":"ID of the vector store to search.","enum":["vs_a","vs_b"]}},"required":["queries"]}}"#;
const STREAM_DISABLED: &str =
    "Streaming is not yet supported for emulated file_search. Disabling stream for this request.";
const GENERATED_ID: &str = "0123456789abcdef0123456789abcdef";
const CALL_ID: &str = "fs_0123456789abcdef01234567";
const REPORT_JSON: &str = r#"{"filename":"report.txt","file_id":"file_1","score":0.87654,"content":[{"type":"text","text":"hello"},{"text":"world"}]}"#;
const RECORD_JSON: &str = r#"{"file_id":"file_1","filename":"report.txt","score":0.87654,"text":"hello world","attributes":{}}"#;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture literal is valid JSON")
}

fn fields(text: &str) -> Fields {
    let Json::Object(fields) = json(text) else {
        panic!("fixture {text} is an object");
    };
    fields
}

fn result(text: &str) -> VectorStoreSearchResult {
    serde_json::from_str(text).expect("fixture result decodes")
}

fn report() -> VectorStoreSearchResult {
    result(REPORT_JSON)
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

fn seconds(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture clock is a decimal")
}

// 1. Tool conversion

#[test]
fn tool_schema() {
    let tool = build_function_tool(&names(&["vs_a", "vs_b"]));
    assert_eq!(
        serde_json::to_string(&tool).expect("the tool serializes"),
        TOOL_JSON
    );
    let carried: Json = wire(&tool).expect("the tool is JSON");
    assert_eq!(carried, json(TOOL_JSON));
}

#[test]
fn tool_replacement() {
    no_file_search_tools();
    other_tools_stay();
    one_file_search_tool();
    empty_vector_store_ids();
    non_object_tools_stay();
    first_seen_order();
    unknown_ids_type();
}

fn no_file_search_tools() {
    let empty: (Vec<Json>, Vec<String>) = (Vec::new(), Vec::new());
    assert_eq!(replace_file_search_tools(None).expect("none"), empty);
    assert_eq!(
        replace_file_search_tools(Some(&[])).expect("an empty list"),
        empty
    );
}

fn other_tools_stay() {
    let fixture = json(r#"[{"type":"custom"},{"name":"x"},{"vector_store_ids":["vs_a"]}]"#);
    let Json::List(others) = fixture else {
        panic!("the fixture is a tool list");
    };
    let (kept, ids) = replace_file_search_tools(Some(&others)).expect("other tools stay");
    assert_eq!(kept, others);
    assert!(ids.is_empty());
}

fn one_file_search_tool() {
    let fixture = json(r#"[{"type":"file_search","vector_store_ids":["vs_b","vs_a","vs_b"]}]"#);
    let Json::List(single) = fixture else {
        panic!("the fixture is a tool list");
    };
    let (kept, ids) = replace_file_search_tools(Some(&single)).expect("one file_search tool");
    let replacement: Json =
        wire(&build_function_tool(&names(&["vs_b", "vs_a"]))).expect("the tool is JSON");
    assert_eq!(kept, vec![replacement]);
    assert_eq!(ids, names(&["vs_b", "vs_a"]));
}

fn empty_vector_store_ids() {
    let fixture = json(
        r#"[{"type":"file_search"},{"type":"file_search","vector_store_ids":null},{"type":"file_search","vector_store_ids":[]},{"type":"file_search","vector_store_ids":false},{"type":"file_search","vector_store_ids":0},{"type":"file_search","vector_store_ids":""}]"#,
    );
    let Json::List(tools) = fixture else {
        panic!("the fixture is a tool list");
    };
    let empty: (Vec<Json>, Vec<String>) = (Vec::new(), Vec::new());
    assert_eq!(
        replace_file_search_tools(Some(&tools)).expect("no ids"),
        empty
    );
}

fn non_object_tools_stay() {
    let fixture = json(r#"[42,"text",[1],{"type":"file_search","vector_store_ids":["vs_a"]}]"#);
    let Json::List(mixed) = fixture else {
        panic!("the fixture is a tool list");
    };
    let (kept, ids) = replace_file_search_tools(Some(&mixed)).expect("mixed tools");
    let tool: Json = wire(&build_function_tool(&names(&["vs_a"]))).expect("the tool is JSON");
    assert_eq!(kept, vec![json("42"), json(r#""text""#), json("[1]"), tool]);
    assert_eq!(ids, names(&["vs_a"]));
}

fn first_seen_order() {
    let fixture = json(
        r#"[{"type":"file_search","vector_store_ids":["vs_a"]},{"type":"custom"},{"type":"file_search","vector_store_ids":["vs_b","vs_a"]}]"#,
    );
    let Json::List(ordered) = fixture else {
        panic!("the fixture is a tool list");
    };
    let (kept, ids) = replace_file_search_tools(Some(&ordered)).expect("ordered tools");
    let tool: Json =
        wire(&build_function_tool(&names(&["vs_a", "vs_b"]))).expect("the tool is JSON");
    assert_eq!(kept, vec![json(r#"{"type":"custom"}"#), tool]);
    assert_eq!(ids, names(&["vs_a", "vs_b"]));
}

fn unknown_ids_type() {
    for fixture in [
        json(r#"[{"type":"file_search","vector_store_ids":"vs_x"}]"#),
        json(r#"[{"type":"file_search","vector_store_ids":["vs_a",1]}]"#),
    ] {
        let Json::List(tools) = fixture else {
            panic!("the fixture is a tool list");
        };
        assert!(matches!(
            replace_file_search_tools(Some(&tools)),
            Err(FileSearchToolsError::WrongType {
                field: "vector_store_ids",
                ..
            })
        ));
    }
}

// 2. Result formatting

#[test]
fn result_rendering() {
    field_lookup();
    joined_text();
    formatted_output();
    include_records();
}

fn field_lookup() {
    let lookup = fields(r#"{"file_id":"file_1"}"#);
    assert_eq!(field(&lookup, "file_id", &NO_DEFAULT), json(r#""file_1""#));
    assert_eq!(field(&lookup, "filename", &NO_DEFAULT), Json::Null);
    let fallback = json(r#""fallback""#);
    assert_eq!(field(&lookup, "filename", &fallback), json(r#""fallback""#));
    assert_eq!(
        field(&fields(r#"{"file_id":null}"#), "file_id", &fallback),
        Json::Null
    );
}

fn joined_text() {
    assert_eq!(joined_content_text(&report()), "hello world");
    assert_eq!(joined_content_text(&result(r#"{}"#)), "");
    assert_eq!(joined_content_text(&result(r#"{"content":[]}"#)), "");
    assert_eq!(
        joined_content_text(&result(r#"{"content":[{"text":""},{"text":"a"},{}]}"#)),
        "a"
    );
}

fn formatted_output() {
    assert_eq!(format_results_as_tool_output(&[]), NO_RESULTS);
    assert_eq!(NO_RESULTS, "No results found in the vector store.");
    assert_eq!(
        format_results_as_tool_output(&[report()]),
        "[Result 1 | report.txt | file_id=file_1 | score=0.877]\nhello world"
    );
    assert_eq!(
        format_results_as_tool_output(&[
            report(),
            result(r#"{"filename":"","file_id":"","score":0,"content":[]}"#)
        ]),
        "[Result 1 | report.txt | file_id=file_1 | score=0.877]\nhello world\n\n[Result 2 | score=0.000]\n"
    );
    assert_eq!(
        format_results_as_tool_output(&[result(
            r#"{"filename":"n","file_id":"f","score":null,"content":null}"#
        )]),
        "[Result 1 | n | file_id=f]\n"
    );
}

fn include_records() {
    let repeated: Json = wire(&search_results_for_include(&[report(), report()]).expect("include"))
        .expect("the include records are JSON");
    assert_eq!(repeated, json(&format!("[{RECORD_JSON},{RECORD_JSON}]")));
    let defaults: Json =
        wire(&search_results_for_include(&[result(r#"{"attributes":null}"#)]).expect("include"))
            .expect("the include record is JSON");
    assert_eq!(
        defaults,
        json(r#"[{"file_id":"","filename":"","score":null,"text":"","attributes":{}}]"#)
    );
    let attributes: Json = wire(
        &search_results_for_include(&[result(r#"{"file_id":"f","attributes":{"key":"value"}}"#)])
            .expect("include"),
    )
    .expect("the include record is JSON");
    assert_eq!(
        attributes,
        json(
            r#"[{"file_id":"f","filename":"","score":null,"text":"","attributes":{"key":"value"}}]"#
        )
    );
    let empty_attributes: Json = wire(
        &search_results_for_include(&[result(r#"{"file_id":"f","attributes":{}}"#)])
            .expect("include"),
    )
    .expect("the include record is JSON");
    assert_eq!(
        empty_attributes,
        json(r#"[{"file_id":"f","filename":"","score":null,"text":"","attributes":{}}]"#)
    );
}

// 3. Output items

#[test]
fn output_items() {
    call_without_results();
    call_with_results();
    message_annotations();
}

fn call_output(results: Option<&[VectorStoreSearchResult]>, include: bool) -> Json {
    let output = build_file_search_call_output(CALL_ID, &names(&["q1"]), results, include)
        .expect("the call output builds");
    wire(&output).expect("the call output is JSON")
}

fn call_without_results() {
    let absent = json(&format!(
        r#"{{"type":"file_search_call","id":"{CALL_ID}","status":"completed","queries":["q1"],"search_results":null}}"#
    ));
    assert_eq!(call_output(None, false), absent);
    assert_eq!(call_output(Some(&[]), false), absent);
    assert_eq!(call_output(Some(&[]), true), absent);
    assert_eq!(call_output(Some(&[report()]), false), absent);
}

fn call_with_results() {
    let included = json(
        r#"{"type":"file_search_call","id":"fs_0123456789abcdef01234567","status":"completed","queries":["q1"],"search_results":[{"file_id":"file_1","filename":"report.txt","score":0.87654,"text":"hello world","attributes":{}}]}"#,
    );
    assert_eq!(call_output(Some(&[report()]), true), included);
}

fn citation_results() -> Vec<VectorStoreSearchResult> {
    vec![
        result(r#"{"filename":"b.txt"}"#),
        result(r#"{"file_id":"file_1","filename":"a.txt"}"#),
        result(r#"{"file_id":"file_1","filename":"c.txt"}"#),
        result(r#"{"file_id":"file_2"}"#),
    ]
}

fn message_annotations() {
    let citations = wire(&citation_annotations(&citation_results(), "answer")).expect("json");
    assert_eq!(
        citations,
        json(
            r#"[{"type":"file_citation","index":6,"file_id":"file_1","filename":"a.txt"},{"type":"file_citation","index":6,"file_id":"file_2","filename":""}]"#
        )
    );
    let empty_text = wire(&citation_annotations(&citation_results(), "")).expect("json");
    assert_eq!(
        empty_text,
        json(
            r#"[{"type":"file_citation","index":0,"file_id":"file_1","filename":"a.txt"},{"type":"file_citation","index":0,"file_id":"file_2","filename":""}]"#
        )
    );
    let message = wire(&build_message_output("answer", &citation_results())).expect("json");
    assert_eq!(
        message,
        json(
            r#"{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer","annotations":[{"type":"file_citation","index":6,"file_id":"file_1","filename":"a.txt"},{"type":"file_citation","index":6,"file_id":"file_2","filename":""}]}]}"#
        )
    );
}

// 4. Response synthesis

fn extracted(output: &str) -> String {
    let items: Vec<ResponseOutputItem> = serde_json::from_str(output).expect("the output decodes");
    response_output_text(&items).expect("the output text extracts")
}

fn usage() -> ResponseApiUsage {
    serde_json::from_str(r#"{"input_tokens":1,"output_tokens":2,"total_tokens":3}"#)
        .expect("the usage decodes")
}

fn outputs() -> OutputItems {
    OutputItems {
        call: build_file_search_call_output(CALL_ID, &names(&["q1"]), None, false)
            .expect("the call output builds"),
        message: build_message_output("answer", &[]),
    }
}

fn original(hidden: Option<&str>) -> OriginalResponse {
    OriginalResponse {
        id: Some("resp_original".to_owned()),
        created_at: Some(1_700_000_000),
        model: OriginalModel::Text("gpt-test".to_owned()),
        usage: Some(usage()),
        hidden_params: hidden.map(fields),
    }
}

fn copied_fields() -> SynthesizedResponse {
    let uuids = Arc::new(AtomicUsize::new(0));
    let clocks = Arc::new(AtomicUsize::new(0));
    let first = fields(r#"{"response_cost":1.25}"#);
    let synthesized = synthesize_response(
        &original(Some(r#"{"response_cost":2.5}"#)),
        &outputs(),
        Some(&first),
        || {
            uuids.fetch_add(1, Ordering::Relaxed);
            GENERATED_ID.to_owned()
        },
        || {
            clocks.fetch_add(1, Ordering::Relaxed);
            seconds("1700000000.75")
        },
    )
    .expect("the response synthesizes");
    assert_eq!(uuids.load(Ordering::Relaxed), 1);
    assert_eq!(clocks.load(Ordering::Relaxed), 1);
    synthesized
}

fn merged_hidden(
    original_hidden: Option<&str>,
    first_hidden: Option<&str>,
) -> Result<HiddenParams, FileSearchToolsError> {
    let first = first_hidden.map(fields);
    synthesize_response(
        &original(original_hidden),
        &outputs(),
        first.as_ref(),
        || GENERATED_ID.to_owned(),
        || seconds("1700000000.75"),
    )
    .map(|synthesized| synthesized.hidden_params)
}

#[test]
fn response_synthesis() {
    output_text_blocks();
    output_text_selection();
    original_fields_copied();
    synthesized_output();
    generated_defaults();
    null_model();
    hidden_merge_sums();
    hidden_merge_copies();
    clock_overflow();
    cost_overflow();
}

fn clock_overflow() {
    let error = synthesize_response(
        &OriginalResponse::default(),
        &outputs(),
        None,
        || GENERATED_ID.to_owned(),
        || Decimal::MAX,
    )
    .expect_err("the clock must fit the signed 64-bit seconds");
    assert!(matches!(error, FileSearchToolsError::ClockOutOfRange));
}

fn cost_overflow() {
    let maximum = r#"{"response_cost":79228162514264337593543950335}"#;
    let error = merged_hidden(Some(maximum), Some(maximum)).expect_err("the sum overflows");
    assert!(matches!(error, FileSearchToolsError::CostOverflow));
    assert!(matches!(
        merged_hidden(
            Some(r#"{"response_cost":1e999}"#),
            Some(r#"{"response_cost":1}"#)
        ),
        Err(FileSearchToolsError::ResponseCost { .. })
    ));
}

fn output_text_blocks() {
    assert_eq!(
        extracted(r#"[{"type":"message","content":[{"type":"output_text","text":"abc"}]}]"#),
        "abc"
    );
    assert_eq!(
        extracted(r#"[{"type":"message","content":[{"type":"output_text","text":null}]}]"#),
        ""
    );
    assert_eq!(
        extracted(r#"[{"type":"message","content":[{"type":"output_text","text":42}]}]"#),
        "42"
    );
}

fn output_text_selection() {
    assert_eq!(
        extracted(
            r#"[{"type":"function_call"},{"type":"message","content":[{"type":"text","text":"no"},{"type":"output_text","text":"yes"}]}]"#
        ),
        "yes"
    );
    assert_eq!(
        extracted(
            r#"[{"type":"message","content":[{"type":"output_text","text":"first"},{"type":"output_text","text":"second"}]}]"#
        ),
        "first"
    );
    assert_eq!(
        extracted(
            r#"[{"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{"type":"output_text","text":"sdk","annotations":[]}]}]"#
        ),
        "sdk"
    );
    assert_eq!(extracted("[]"), "");
    output_text_typed();
    output_text_errors();
}

fn typed_message(part_type: Option<&str>, text: Option<&str>) -> ResponseOutputItem {
    ResponseOutputItem::Message(GenericResponseOutputItem {
        r#type: "message".to_owned(),
        id: "msg_typed".to_owned(),
        status: "completed".to_owned(),
        role: "assistant".to_owned(),
        content: vec![OutputText {
            r#type: part_type.map(str::to_owned),
            text: text.map(str::to_owned),
            annotations: None,
            extra: BTreeMap::new(),
        }],
        phase: None,
        extra: BTreeMap::new(),
    })
}

fn output_text_typed() {
    let typed = vec![typed_message(Some("output_text"), Some("typed"))];
    assert_eq!(
        response_output_text(&typed).expect("the typed message extracts"),
        "typed"
    );
    let typed_null = vec![typed_message(Some("output_text"), None)];
    assert_eq!(
        response_output_text(&typed_null).expect("a null typed text is empty"),
        ""
    );
    let other_part = vec![typed_message(Some("text"), Some("not output text"))];
    assert_eq!(
        response_output_text(&other_part).expect("a non output_text part is skipped"),
        ""
    );
    let function_call = vec![ResponseOutputItem::FunctionCall(OutputFunctionToolCall {
        arguments: None,
        call_id: None,
        name: None,
        r#type: None,
        id: None,
        status: FunctionCallStatus::Completed,
        phase: None,
        extra: BTreeMap::new(),
    })];
    assert_eq!(
        response_output_text(&function_call).expect("a function call has no text"),
        ""
    );
}

fn output_text_errors() {
    let bad_content = vec![
        serde_json::from_str::<ResponseOutputItem>(r#"{"type":"message","content":"blocks"}"#)
            .expect("the item decodes"),
    ];
    assert!(matches!(
        response_output_text(&bad_content),
        Err(FileSearchToolsError::WrongType {
            field: "content",
            ..
        })
    ));
    let bad_text = vec![
        serde_json::from_str::<ResponseOutputItem>(
            r#"{"type":"message","content":[{"type":"output_text","text":[]}]}"#,
        )
        .expect("the item decodes"),
    ];
    assert!(matches!(
        response_output_text(&bad_text),
        Err(FileSearchToolsError::WrongType { field: "text", .. })
    ));
    assert_eq!(
        extracted(r#"[{"type":"message","content":[42,{"type":"output_text","text":"block"}]}]"#),
        "block"
    );
    assert_eq!(extracted(r#"[{"type":"message","content":null}]"#), "");
}

fn original_fields_copied() {
    let synthesized = copied_fields();
    assert_eq!(synthesized.response.id, "resp_original");
    assert_eq!(synthesized.response.created_at, 1_700_000_000);
    assert_eq!(synthesized.response.model.as_deref(), Some("gpt-test"));
    assert_eq!(synthesized.response.usage, Some(usage()));
    assert_eq!(synthesized.response.object.as_deref(), Some("response"));
    assert_eq!(synthesized.response.status.as_deref(), Some("completed"));
    assert_eq!(synthesized.response.error, None);
}

fn synthesized_output() {
    let synthesized = copied_fields();
    let output: Json = wire(&synthesized.response.output).expect("the output is JSON");
    assert_eq!(
        output,
        json(&format!(
            r#"[{{"type":"file_search_call","id":"{CALL_ID}","queries":["q1"],"status":"completed","results":null,"search_results":null}},{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"answer","annotations":[]}}]}}]"#
        ))
    );
    assert_eq!(
        synthesized.hidden_params,
        fields(r#"{"response_cost":3.75}"#)
    );
}

fn generated_defaults() {
    let fallback = synthesize_response(
        &OriginalResponse::default(),
        &outputs(),
        None,
        || GENERATED_ID.to_owned(),
        || seconds("1700000000.75"),
    )
    .expect("the response synthesizes");
    assert_eq!(fallback.response.id, format!("resp_{GENERATED_ID}"));
    assert_eq!(fallback.response.created_at, 1_700_000_000);
    assert_eq!(fallback.response.model.as_deref(), Some(""));
    assert_eq!(fallback.response.usage, None);
    assert_eq!(fallback.response.error, None);
    assert_eq!(fallback.response.status.as_deref(), Some("completed"));
    assert!(fallback.hidden_params.is_empty());
}

fn null_model() {
    let null_model = OriginalResponse {
        id: None,
        created_at: None,
        model: OriginalModel::Null,
        usage: None,
        hidden_params: None,
    };
    let synthesized = synthesize_response(
        &null_model,
        &outputs(),
        None,
        || GENERATED_ID.to_owned(),
        || Decimal::ZERO,
    )
    .expect("the response synthesizes");
    assert_eq!(synthesized.response.model, None);
    assert_eq!(synthesized.response.id, format!("resp_{GENERATED_ID}"));
    assert_eq!(synthesized.response.created_at, 0);
}

fn hidden_merge_sums() {
    assert_eq!(
        merged_hidden(
            Some(r#"{"response_cost":2.5}"#),
            Some(r#"{"response_cost":1.25}"#)
        )
        .expect("the hidden params merge"),
        fields(r#"{"response_cost":3.75}"#)
    );
    assert_eq!(
        merged_hidden(Some(r#"{"other":true}"#), Some(r#"{"response_cost":1.25}"#))
            .expect("the hidden params merge"),
        fields(r#"{"other":true,"response_cost":1.25}"#)
    );
    // a decimal sum is exact here where Python's float sum of 0.1 and 0.2 is 0.30000000000000004
    assert_eq!(
        merged_hidden(
            Some(r#"{"response_cost":0.1}"#),
            Some(r#"{"response_cost":0.2}"#)
        )
        .expect("the decimal sum is exact"),
        fields(r#"{"response_cost":0.3}"#)
    );
    assert_eq!(
        merged_hidden(
            Some(r#"{"response_cost":null}"#),
            Some(r#"{"response_cost":1.25}"#)
        )
        .expect("a null current cost counts as zero"),
        fields(r#"{"response_cost":1.25}"#)
    );
}

fn hidden_merge_copies() {
    assert_eq!(
        merged_hidden(Some(r#"{"response_cost":2.5}"#), None).expect("the hidden params copy"),
        fields(r#"{"response_cost":2.5}"#)
    );
    assert_eq!(
        merged_hidden(Some(r#"{"response_cost":2.5}"#), Some(r#"{}"#)).expect("no first cost"),
        fields(r#"{"response_cost":2.5}"#)
    );
    assert!(
        merged_hidden(None, Some(r#"{"response_cost":1.25}"#))
            .expect("no hidden attribute")
            .is_empty()
    );
    assert_eq!(
        merged_hidden(
            Some(r#"{"response_cost":2.5}"#),
            Some(r#"{"response_cost":null}"#)
        )
        .expect("a null first cost adds nothing"),
        fields(r#"{"response_cost":2.5}"#)
    );
    assert!(matches!(
        merged_hidden(
            Some(r#"{"response_cost":"2.5"}"#),
            Some(r#"{"response_cost":1}"#)
        ),
        Err(FileSearchToolsError::WrongType {
            field: "response_cost",
            ..
        })
    ));
}

// 5. Argument preparation

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<String>>>);

impl CapturedLogs {
    fn take(&self) -> Vec<String> {
        let mut messages = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        mem::take(&mut messages)
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

fn capture<T, F>(call: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let logs = CapturedLogs::default();
    let subscriber = registry().with(logs.clone());
    let value = with_default(subscriber, call);
    (value, logs.take())
}

fn prepared(kwargs: &str) -> bool {
    prepare_file_search_call(&fields(kwargs))
        .expect("the call prepares")
        .include_search_results
}

#[test]
fn argument_preparation() {
    include_flag();
    stream_switched_off();
    call_id_precedence();
    call_arguments();
    query_list();
    query_fallbacks();
}

fn include_flag() {
    assert!(!prepared("{}"));
    assert!(!prepared(r#"{"include":null}"#));
    assert!(!prepared(r#"{"include":[]}"#));
    assert!(prepared(r#"{"include":["file_search_call.results"]}"#));
    assert!(!prepared(r#"{"include":["message.output_text"]}"#));
    assert!(matches!(
        prepare_file_search_call(&fields(r#"{"include":"file_search_call.results"}"#)),
        Err(FileSearchToolsError::WrongType {
            field: "include",
            ..
        })
    ));
}

fn stream_switched_off() {
    let streaming = fields(r#"{"stream":1,"include":["file_search_call.results"]}"#);
    let (call, logs) = capture(|| prepare_file_search_call(&streaming));
    let call = call.expect("the streaming call prepares");
    assert_eq!(call.updated_kwargs.get("stream"), Some(&Json::Bool(false)));
    assert!(call.include_search_results);
    assert_eq!(streaming.get("stream"), Some(&json("1")));
    assert_eq!(logs, vec![STREAM_DISABLED.to_owned()]);
    let quiet = fields(r#"{"stream":0}"#);
    let (call, logs) = capture(|| prepare_file_search_call(&quiet));
    assert_eq!(call.expect("the quiet call prepares").updated_kwargs, quiet);
    assert!(logs.is_empty());
    let (call, logs) = capture(|| prepare_file_search_call(&fields("{}")));
    assert_eq!(
        call.expect("a call without stream prepares").updated_kwargs,
        fields("{}")
    );
    assert!(logs.is_empty());
}

fn call_id_precedence() {
    assert_eq!(
        extract_tool_call_fields(
            &fields(r#"{"call_id":"call_1","id":"id_1","arguments":"{\"queries\":[\"q\"]}"}"#),
            "fb"
        )
        .expect("the call fields extract"),
        (String::from("call_1"), json(r#""{\"queries\":[\"q\"]}""#))
    );
    assert_eq!(
        extract_tool_call_fields(&fields(r#"{"id":"id_2"}"#), "fb").expect("the id wins"),
        (String::from("id_2"), json(r#""{}""#))
    );
    assert_eq!(
        extract_tool_call_fields(&fields("{}"), "fs_fallback").expect("the fallback wins"),
        (String::from("fs_fallback"), json(r#""{}""#))
    );
    assert_eq!(
        extract_tool_call_fields(&fields(r#"{"call_id":"","id":"id_3","arguments":0}"#), "fb")
            .expect("a falsey id falls through"),
        (String::from("id_3"), json(r#""{}""#))
    );
    assert_eq!(
        extract_tool_call_fields(&fields(r#"{"call_id":"5","arguments":1.5}"#), "fb")
            .expect("a numeric id reads as its JSON text"),
        (String::from("5"), json("1.5"))
    );
    assert!(matches!(
        extract_tool_call_fields(&fields(r#"{"call_id":true}"#), "fb"),
        Err(FileSearchToolsError::WrongType {
            field: "call_id",
            ..
        })
    ));
}

fn call_arguments() {
    assert_eq!(
        extract_tool_call_fields(&fields(r#"{"call_id":"c","arguments":null}"#), "fb")
            .expect("a null argument reads as an empty object"),
        (String::from("c"), json(r#""{}""#))
    );
    assert_eq!(
        extract_tool_call_fields(&fields(r#"{"call_id":"c","arguments":""}"#), "fb")
            .expect("an empty argument reads as an empty object"),
        (String::from("c"), json(r#""{}""#))
    );
    assert_eq!(
        extract_tool_call_fields(&fields(r#"{"call_id":"c","arguments":{"a":1}}"#), "fb")
            .expect("a non-string argument is preserved"),
        (String::from("c"), json(r#"{"a":1}"#))
    );
}

fn query_list() {
    let input = json(r#""in""#);
    assert_eq!(
        resolve_queries_from_args(&fields(r#"{"queries":["q1","q2"]}"#), &input).expect("queries"),
        names(&["q1", "q2"])
    );
    assert_eq!(
        resolve_queries_from_args(&fields(r#"{"queries":"q"}"#), &input).expect("one query"),
        names(&["q"])
    );
    assert_eq!(
        resolve_queries_from_args(&fields(r#"{"queries":["a",3]}"#), &input)
            .expect("numbers read as their JSON text"),
        names(&["a", "3"])
    );
    assert!(matches!(
        resolve_queries_from_args(&fields(r#"{"queries":[true]}"#), &input),
        Err(FileSearchToolsError::WrongType {
            field: "queries",
            ..
        })
    ));
}

fn query_fallbacks() {
    assert_eq!(
        resolve_queries_from_args(&fields(r#"{"queries":[],"query":"legacy"}"#), &json("null"))
            .expect("the legacy query"),
        names(&["legacy"])
    );
    assert_eq!(
        resolve_queries_from_args(&fields(r#"{"queries":null,"query":""}"#), &json("42"))
            .expect("the input as one query"),
        names(&["42"])
    );
    assert_eq!(
        resolve_queries_from_args(&fields("{}"), &json("42")).expect("the input as one query"),
        names(&["42"])
    );
    assert_eq!(
        resolve_queries_from_args(&fields(r#"{"queries":[],"query":null}"#), &json("42"))
            .expect("a null query falls through"),
        names(&["42"])
    );
    assert!(matches!(
        resolve_queries_from_args(&fields("{}"), &json("true")),
        Err(FileSearchToolsError::WrongType { field: "input", .. })
    ));
}
