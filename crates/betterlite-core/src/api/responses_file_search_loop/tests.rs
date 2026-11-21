#![cfg(test)]
#![expect(
    clippy::too_many_lines,
    reason = "a behavior test asserts one whole provider sequence, the searches it ran and the response it produced"
)]

use std::collections::VecDeque;
use std::fmt::Debug;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreSearchResponse, VectorStoreSearchResult,
};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::decoder::tool_call_arguments;
use super::error::FileSearchLoopError;
use super::follow_up::build_follow_up_input;
use super::ports::{FollowUpRequest, ResponsesFollowUp, SearchRequest, VectorStoreSearches};
use super::records::{DiagnosticMessage, FollowUpCall, SearchAuthorization, UpstreamSearch};
use super::{
    DecodeError, ResponsesFileSearchLoop, ResponsesFileSearchLoopInput,
    decode_responses_file_search_args,
};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::responses_tool_namespaces::values::Fields;

const MODEL: &str = "gpt-test";
const GENERATED: &str = "0123456789abcdef0123456789abcdef";
const SYNTHESIZED_CALL_ID: &str = "fs_0123456789abcdef01234567";
// a model output item shaped like the ones a provider returns, with a null the model dump drops
const PREVIOUS_FILE_SEARCH: &str = r#"{"type":"file_search_call","id":"fs_prev","queries":["old"],"status":"completed","results":null}"#;

fn parse<T>(text: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).unwrap()
}

fn json(text: &str) -> Json {
    parse(text)
}

fn text(value: &str) -> Json {
    Json::Text(String::from(value))
}

fn field<'value>(value: &'value Json, key: &str) -> &'value Json {
    if let Json::Object(fields) = value
        && let Some(found) = fields.get(key)
    {
        return found;
    }
    &Json::Null
}

fn nth<'value>(list: &'value Json, index: usize) -> &'value Json {
    if let Json::List(items) = list {
        return &items[index];
    }
    &Json::Null
}

fn at<'value>(value: &'value Json, path: &[&str]) -> &'value Json {
    path.iter().fold(value, |current, key| field(current, key))
}

fn contains_key(value: &Json, key: &str) -> bool {
    if let Json::Object(fields) = value {
        return fields.contains_key(key);
    }
    false
}

fn fields_of<'fields>(fields: &'fields Fields, key: &str) -> &'fields Json {
    fields.get(key).unwrap_or(&Json::Null)
}

fn synthesized(response: &ResponsesApiResponse) -> Json {
    parse(&serde_json::to_string(response).unwrap())
}

fn test_authorization() -> SearchAuthorization {
    SearchAuthorization {
        user: Some(String::from("user_1")),
        team: Some(String::from("team_1")),
    }
}

fn loop_input(input: Json, tools: Vec<Json>, kwargs: Fields) -> ResponsesFileSearchLoopInput {
    ResponsesFileSearchLoopInput {
        input,
        model: String::from(MODEL),
        tools: Some(tools),
        kwargs,
        authorization: test_authorization(),
    }
}

fn file_search_loop<'deps>(
    follow_up: &'deps FakeFollowUp,
    searches: &'deps FakeSearches,
) -> ResponsesFileSearchLoop<'deps, FakeFollowUp, FakeSearches> {
    ResponsesFileSearchLoop {
        follow_up,
        searches,
        limits: ApiLimits::default(),
        ids: Arc::new(|| String::from(GENERATED)),
        clock: Arc::new(|| Decimal::from(1_700_000_100_i64)),
    }
}

fn file_search_tool(ids: &[&str]) -> Json {
    let listed = ids.iter().map(|id| text(*id)).collect();
    Json::Object(IndexMap::from([
        (String::from("type"), text("file_search")),
        (String::from("vector_store_ids"), Json::List(listed)),
    ]))
}

fn queries_arguments(queries: &[&str]) -> String {
    let listed: Vec<String> = queries.iter().map(|query| format!("{query:?}")).collect();
    format!("{{\"queries\":[{}]}}", listed.join(","))
}

fn function_call_item(arguments: &str, call_id: &str) -> String {
    format!(
        r#"{{"type":"function_call","name":"litellm_file_search","arguments":{arguments:?},"call_id":{call_id:?},"id":"fc_1"}}"#
    )
}

fn message_item(content: &str) -> String {
    format!(
        r#"{{"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{{"type":"output_text","text":{content:?},"annotations":[]}}]}}"#
    )
}

fn response(id: &str, output: &[String]) -> ResponsesApiResponse {
    parse(&format!(
        r#"{{"id":"{id}","created_at":1700000000,"model":"{MODEL}","status":"completed","output":[{}]}}"#,
        output.join(",")
    ))
}

fn search_result(file_id: &str, filename: &str, content: &str) -> VectorStoreSearchResult {
    parse(&format!(
        r#"{{"file_id":{file_id:?},"filename":{filename:?},"content":[{{"type":"text","text":{content:?}}}]}}"#
    ))
}

#[derive(Debug, Clone, Copy, Error)]
#[error("fake provider failure")]
struct FakeError;

struct FakeFollowUp {
    responses: Mutex<VecDeque<Result<ResponsesApiResponse, FakeError>>>,
    calls: Mutex<Vec<FollowUpCall>>,
}

impl FakeFollowUp {
    fn answering(responses: Vec<ResponsesApiResponse>) -> Self {
        Self::scripted(responses.into_iter().map(Ok).collect())
    }

    fn scripted(responses: Vec<Result<ResponsesApiResponse, FakeError>>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self::scripted(vec![Err(FakeError)])
    }

    fn calls(&self) -> Vec<FollowUpCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ResponsesFollowUp for FakeFollowUp {
    type Error = FakeError;

    async fn call(&self, request: FollowUpRequest<'_>) -> Result<ResponsesApiResponse, FakeError> {
        self.calls.lock().unwrap().push(FollowUpCall {
            input: request.input.clone(),
            model: String::from(request.model),
            tools: request.tools.map(<[Json]>::to_vec),
            kwargs: request.kwargs.clone(),
            internal: request.internal,
        });
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted provider response")
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RecordedSearch {
    vector_store_id: String,
    query: String,
    authorization: SearchAuthorization,
}

struct FakeSearches {
    scripted: Vec<(
        String,
        String,
        Result<Vec<VectorStoreSearchResult>, FakeError>,
    )>,
    requests: Mutex<Vec<RecordedSearch>>,
}

impl FakeSearches {
    fn scripted(
        scripted: Vec<(&str, &str, Result<Vec<VectorStoreSearchResult>, FakeError>)>,
    ) -> Self {
        let scripted = scripted
            .into_iter()
            .map(|(store, query, found)| (String::from(store), String::from(query), found))
            .collect();
        Self {
            scripted,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<RecordedSearch> {
        self.requests.lock().unwrap().clone()
    }

    fn requested_stores(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|request| request.vector_store_id)
            .collect()
    }

    fn scripted_result(
        &self,
        vector_store_id: &str,
        query: &str,
    ) -> Result<Vec<VectorStoreSearchResult>, FakeError> {
        for (store, name, found) in &self.scripted {
            if store == vector_store_id && name == query {
                return found.clone();
            }
        }
        Err(FakeError)
    }
}

#[async_trait]
impl VectorStoreSearches for FakeSearches {
    type Error = FakeError;

    async fn asearch(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<VectorStoreSearchResponse, FakeError> {
        self.requests.lock().unwrap().push(RecordedSearch {
            vector_store_id: String::from(request.vector_store_id),
            query: String::from(request.query),
            authorization: request.authorization.clone(),
        });
        let found = self.scripted_result(request.vector_store_id, request.query)?;
        Ok(VectorStoreSearchResponse {
            data: Some(found),
            ..VectorStoreSearchResponse::default()
        })
    }
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<LoggedEvent>>>);

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoggedEvent {
    level: String,
    target: String,
    message: String,
}

impl Capture {
    fn events(&self) -> Vec<LoggedEvent> {
        self.0.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
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

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        self.0.lock().unwrap().push(LoggedEvent {
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_owned(),
            message: visitor.0,
        });
    }
}

#[tokio::test]
async fn file_search_follow_up() {
    let follow_up = FakeFollowUp::answering(vec![
        response(
            "resp_first",
            &[
                function_call_item(&queries_arguments(&["alpha"]), "call_1"),
                message_item("before search"),
                String::from(PREVIOUS_FILE_SEARCH),
            ],
        ),
        response("resp_final", &[message_item("The answer is 42.")]),
    ]);
    let searches = FakeSearches::scripted(vec![(
        "vs_1",
        "alpha",
        Ok(vec![search_result("f1", "a.txt", "alpha text")]),
    )]);
    let input = loop_input(
        json(r#"[{"role":"user","content":"question"}]"#),
        vec![file_search_tool(&["vs_1"])],
        Fields::new(),
    );
    let result = file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap();

    assert!(result.diagnostics.is_empty());
    assert!(result.hidden_params.is_empty());
    let calls = follow_up.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].internal);
    assert_eq!(calls[0].model, MODEL);
    assert_eq!(
        at(&calls[0].tools.as_ref().unwrap()[0], &["name"]),
        &text("litellm_file_search")
    );
    assert!(calls[1].tools.is_none());
    assert!(calls[1].internal);
    assert_eq!(calls[1].kwargs, calls[0].kwargs);

    let Json::List(items) = &calls[1].input else {
        panic!("the follow-up input is a list")
    };
    assert_eq!(items.len(), 5);
    assert_eq!(items[0], json(r#"{"role":"user","content":"question"}"#));
    assert_eq!(at(&items[1], &["call_id"]), &text("call_1"));
    assert!(!contains_key(&items[1], "caller"));
    assert!(!contains_key(&items[3], "results"));
    assert_eq!(at(&items[4], &["type"]), &text("function_call_output"));
    assert!(
        matches!(at(&items[4], &["output"]), Json::Text(output) if output.contains("alpha text"))
    );

    let body = synthesized(&result.response);
    assert_eq!(at(&body, &["id"]), &text("resp_final"));
    let output = at(&body, &["output"]);
    assert_eq!(at(nth(output, 0), &["id"]), &text(SYNTHESIZED_CALL_ID));
    assert_eq!(at(nth(output, 0), &["queries"]), &json(r#"["alpha"]"#));
    assert_eq!(
        at(nth(output, 1), &["content"]),
        &json(
            r#"[{"type":"output_text","text":"The answer is 42.","annotations":[{"type":"file_citation","index":17,"file_id":"f1","filename":"a.txt"}]}]"#
        )
    );
}

#[tokio::test]
async fn multiple_vector_stores() {
    let follow_up = FakeFollowUp::answering(vec![
        response(
            "resp_first",
            &[function_call_item(&queries_arguments(&["q"]), "call_1")],
        ),
        response("resp_final", &[message_item("done")]),
    ]);
    let searches = FakeSearches::scripted(vec![
        ("vs_1", "q", Ok(vec![search_result("f1", "a.txt", "one")])),
        ("vs_2", "q", Ok(vec![search_result("f2", "b.txt", "two")])),
        ("vs_3", "q", Ok(Vec::new())),
    ]);
    let tools = vec![
        file_search_tool(&["vs_1", "vs_2"]),
        file_search_tool(&["vs_2", "vs_3"]),
    ];
    let input = loop_input(json(r#""question""#), tools, Fields::new());
    let result = file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap();

    assert_eq!(
        searches.requested_stores(),
        ["vs_1", "vs_2", "vs_3"].map(String::from)
    );
    let calls = follow_up.calls();
    assert_eq!(
        at(
            &calls[0].tools.as_ref().unwrap()[0],
            &["parameters", "properties", "vector_store_id", "enum"]
        ),
        &json(r#"["vs_1","vs_2","vs_3"]"#)
    );
    let Json::List(items) = &calls[1].input else {
        panic!("the follow-up input is a list")
    };
    let output = match at(&items[2], &["output"]) {
        Json::Text(output) => output,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("the tool output is text")
        }
    };
    assert!(output.contains("a.txt"));
    assert!(output.contains("b.txt"));
    assert_eq!(result.searches.len(), 3);
    assert_eq!(result.searches[2].result_count, 0);
}

#[tokio::test]
async fn no_tool_call_final() {
    let follow_up = FakeFollowUp::answering(vec![response(
        "resp_first",
        &[message_item("Direct answer")],
    )]);
    let searches = FakeSearches::scripted(Vec::new());
    let input = loop_input(
        json(r#""question""#),
        vec![file_search_tool(&["vs_1"])],
        Fields::new(),
    );
    let result = file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap();

    assert!(searches.requests().is_empty());
    assert_eq!(follow_up.calls().len(), 1);
    assert!(result.searches.is_empty());
    assert!(result.diagnostics.is_empty());
    assert_eq!(result.response.id, "resp_first");
    let body = synthesized(&result.response);
    let output = at(&body, &["output"]);
    assert_eq!(at(nth(output, 0), &["id"]), &text(SYNTHESIZED_CALL_ID));
    assert_eq!(at(nth(output, 0), &["queries"]), &json(r#"["question"]"#));
    assert_eq!(at(nth(output, 0), &["search_results"]), &Json::Null);
    assert_eq!(
        at(nth(output, 1), &["content"]),
        &json(r#"[{"type":"output_text","text":"Direct answer","annotations":[]}]"#)
    );
}

#[tokio::test]
async fn failed_search_outcome() {
    let follow_up = FakeFollowUp::answering(vec![
        response(
            "resp_first",
            &[function_call_item(&queries_arguments(&["q"]), "call_1")],
        ),
        response("resp_final", &[message_item("after")]),
    ]);
    let searches = FakeSearches::scripted(vec![
        ("vs_bad", "q", Err(FakeError)),
        (
            "vs_good",
            "q",
            Ok(vec![search_result("f1", "a.txt", "kept text")]),
        ),
    ]);
    let input = loop_input(
        json(r#""question""#),
        vec![file_search_tool(&["vs_bad", "vs_good"])],
        Fields::new(),
    );
    let capture = Capture::default();
    let guard = set_default(registry().with(capture.clone()));
    let result = file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap();
    drop(guard);

    let warning = "file_search emulated: search failed for query='q', vector_store_id='vs_bad': fake provider failure";
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, "WARNING");
    assert_eq!(result.diagnostics[0].message, warning);
    assert!(capture.events().iter().any(|event| {
        event.target == "LiteLLM" && event.level == "WARN" && event.message == warning
    }));
    assert!(result.searches[0].failed);
    assert_eq!(
        result.searches[1],
        UpstreamSearch {
            vector_store_id: String::from("vs_good"),
            query: String::from("q"),
            result_count: 1,
            failed: false,
        }
    );
    let body = synthesized(&result.response);
    assert_eq!(
        at(nth(at(&body, &["output"]), 1), &["content"]),
        &json(
            r#"[{"type":"output_text","text":"after","annotations":[{"type":"file_citation","index":5,"file_id":"f1","filename":"a.txt"}]}]"#
        )
    );
}

#[tokio::test]
async fn stream_synthetic_result() {
    let follow_up = FakeFollowUp::answering(vec![
        response(
            "resp_first",
            &[function_call_item(&queries_arguments(&["alpha"]), "call_1")],
        ),
        response("resp_final", &[message_item("answer")]),
    ]);
    let searches = FakeSearches::scripted(vec![(
        "vs_1",
        "alpha",
        Ok(vec![search_result("f1", "a.txt", "alpha text")]),
    )]);
    let kwargs = IndexMap::from([
        (String::from("stream"), Json::Bool(true)),
        (
            String::from("include"),
            json(r#"["file_search_call.results"]"#),
        ),
    ]);
    let input = loop_input(
        json(r#""question""#),
        vec![file_search_tool(&["vs_1"])],
        kwargs,
    );
    let result = file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap();

    let calls = follow_up.calls();
    assert_eq!(fields_of(&calls[0].kwargs, "stream"), &Json::Bool(false));
    assert_eq!(fields_of(&calls[1].kwargs, "stream"), &Json::Bool(false));
    assert_eq!(
        fields_of(&calls[0].kwargs, "include"),
        &json(r#"["file_search_call.results"]"#)
    );
    let body = synthesized(&result.response);
    let search_item = nth(at(&body, &["output"]), 0);
    assert_eq!(
        at(search_item, &["search_results"]),
        &json(
            r#"[{"file_id":"f1","filename":"a.txt","score":null,"text":"alpha text","attributes":{}}]"#
        )
    );
}

#[tokio::test]
async fn every_search_carries_authorization() {
    let follow_up = FakeFollowUp::answering(vec![
        response(
            "resp_first",
            &[function_call_item(
                &queries_arguments(&["q1", "q2"]),
                "call_1",
            )],
        ),
        response("resp_final", &[message_item("done")]),
    ]);
    let searches = FakeSearches::scripted(vec![
        ("vs_1", "q1", Ok(Vec::new())),
        ("vs_2", "q1", Ok(Vec::new())),
        ("vs_1", "q2", Ok(Vec::new())),
        ("vs_2", "q2", Ok(Vec::new())),
    ]);
    let input = loop_input(
        json(r#""question""#),
        vec![file_search_tool(&["vs_1", "vs_2"])],
        Fields::new(),
    );
    file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap();

    let requests = searches.requests();
    assert_eq!(requests.len(), 4);
    assert!(
        requests
            .iter()
            .all(|request| request.authorization == test_authorization())
    );
    let ordered: Vec<(String, String)> = requests
        .into_iter()
        .map(|request| (request.vector_store_id, request.query))
        .collect();
    assert_eq!(
        ordered,
        vec![
            (String::from("vs_1"), String::from("q1")),
            (String::from("vs_2"), String::from("q1")),
            (String::from("vs_1"), String::from("q2")),
            (String::from("vs_2"), String::from("q2")),
        ]
    );
}

#[tokio::test]
async fn unlisted_store_searches_request_stores() {
    let arguments = String::from(r#"{"queries":["q"],"vector_store_id":"vs_x"}"#);
    let follow_up = FakeFollowUp::answering(vec![
        response("resp_first", &[function_call_item(&arguments, "call_1")]),
        response("resp_final", &[message_item("done")]),
    ]);
    let searches = FakeSearches::scripted(vec![("vs_1", "q", Ok(Vec::new()))]);
    let input = loop_input(
        json(r#""question""#),
        vec![file_search_tool(&["vs_1"])],
        Fields::new(),
    );
    let result = file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap();

    assert_eq!(searches.requested_stores(), ["vs_1"].map(String::from));
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(
        result.diagnostics[0].message,
        "file_search emulated: model picked vector_store_id='vs_x' outside the request's vector_store_ids ['vs_1']; searching the request's stores instead"
    );
}

#[tokio::test]
async fn listed_store_narrows_the_search() {
    let arguments = String::from(r#"{"queries":["q"],"vector_store_id":"vs_2"}"#);
    let follow_up = FakeFollowUp::answering(vec![
        response("resp_first", &[function_call_item(&arguments, "call_1")]),
        response("resp_final", &[message_item("done")]),
    ]);
    let searches = FakeSearches::scripted(vec![("vs_2", "q", Ok(Vec::new()))]);
    let input = loop_input(
        json(r#""question""#),
        vec![file_search_tool(&["vs_1", "vs_2"])],
        Fields::new(),
    );
    let result = file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap();

    assert_eq!(searches.requested_stores(), ["vs_2"].map(String::from));
    assert!(result.diagnostics.is_empty());
}

#[test]
fn store_warning_quotes_like_python() {
    let known = [String::from("vs_1")];
    let both_value = text(r#"a'"b"#);
    let both = DiagnosticMessage::StoreOutside {
        value: &both_value,
        known: &known,
    }
    .to_string();
    assert!(both.contains(r#"vector_store_id='a\'"b' outside"#));
    let single_value = text("a'b");
    let single = DiagnosticMessage::StoreOutside {
        value: &single_value,
        known: &known,
    }
    .to_string();
    assert!(single.contains(r#"vector_store_id="a'b" outside"#));
}

#[tokio::test]
async fn two_tool_calls_format_their_own_results() {
    let follow_up = FakeFollowUp::answering(vec![
        response(
            "resp_first",
            &[
                function_call_item(&queries_arguments(&["alpha"]), "call_1"),
                function_call_item(&queries_arguments(&["beta"]), "call_2"),
            ],
        ),
        response("resp_final", &[message_item("done")]),
    ]);
    let searches = FakeSearches::scripted(vec![
        (
            "vs_1",
            "alpha",
            Ok(vec![search_result("f1", "a.txt", "first result text")]),
        ),
        (
            "vs_1",
            "beta",
            Ok(vec![search_result("f2", "b.txt", "second result text")]),
        ),
    ]);
    let input = loop_input(
        json(r#""question""#),
        vec![file_search_tool(&["vs_1"])],
        Fields::new(),
    );
    let result = file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap();

    let calls = follow_up.calls();
    let Json::List(items) = &calls[1].input else {
        panic!("the follow-up input is a list")
    };
    let (Some(first), Some(second)) = (tool_output(items, 3), tool_output(items, 4)) else {
        panic!("one function_call_output per tool call")
    };
    assert!(first.contains("first result text"));
    assert!(!first.contains("second result text"));
    assert!(second.contains("second result text"));
    assert_eq!(result.searches.len(), 2);
}

#[tokio::test]
async fn other_function_calls_are_not_searched() {
    let arguments = queries_arguments(&["q"]);
    let unrelated = format!(
        r#"{{"type":"function_call","name":"other_tool","arguments":{arguments:?},"call_id":"call_1","id":"fc_1"}}"#
    );
    let follow_up = FakeFollowUp::answering(vec![response("resp_first", &[unrelated])]);
    let searches = FakeSearches::scripted(Vec::new());
    let input = loop_input(
        json(r#""question""#),
        vec![file_search_tool(&["vs_1"])],
        Fields::new(),
    );
    let result = file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap();

    assert!(searches.requests().is_empty());
    assert_eq!(follow_up.calls().len(), 1);
    let body = synthesized(&result.response);
    let search_item = nth(at(&body, &["output"]), 0);
    assert_eq!(at(search_item, &["queries"]), &json(r#"["question"]"#));
    assert_eq!(at(search_item, &["search_results"]), &Json::Null);
}

fn tool_output(items: &[Json], index: usize) -> Option<&str> {
    if let Json::Text(output) = field(&items[index], "output") {
        return Some(output);
    }
    None
}

#[tokio::test]
async fn broken_arguments_search_the_input() {
    let follow_up = FakeFollowUp::answering(vec![
        response("resp_first", &[function_call_item("{oops", "call_1")]),
        response("resp_final", &[message_item("done")]),
    ]);
    let searches = FakeSearches::scripted(vec![("vs_1", "question", Ok(Vec::new()))]);
    let input = loop_input(
        json(r#""question""#),
        vec![file_search_tool(&["vs_1"])],
        Fields::new(),
    );
    file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap();

    let requests = searches.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].query, "question");
}

#[tokio::test]
async fn first_call_failure_is_returned() {
    let follow_up = FakeFollowUp::failing();
    let searches = FakeSearches::scripted(Vec::new());
    let input = loop_input(
        json(r#""question""#),
        vec![file_search_tool(&["vs_1"])],
        Fields::new(),
    );
    let error = file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap_err();
    assert!(matches!(error, FileSearchLoopError::FirstCall(FakeError)));
    assert!(searches.requests().is_empty());
}

#[tokio::test]
async fn follow_up_call_failure_is_returned() {
    let follow_up = FakeFollowUp::scripted(vec![
        Ok(response(
            "resp_first",
            &[function_call_item(&queries_arguments(&["q"]), "call_1")],
        )),
        Err(FakeError),
    ]);
    let searches = FakeSearches::scripted(vec![("vs_1", "q", Ok(Vec::new()))]);
    let input = loop_input(
        json(r#""question""#),
        vec![file_search_tool(&["vs_1"])],
        Fields::new(),
    );
    let error = file_search_loop(&follow_up, &searches)
        .run(&input)
        .await
        .unwrap_err();
    assert!(matches!(error, FileSearchLoopError::FollowUp(FakeError)));
}

#[test]
fn arguments_text_is_decoded() {
    let fields =
        tool_call_arguments(&text(&queries_arguments(&["alpha"])), &ApiLimits::default()).unwrap();
    assert_eq!(fields.get("queries"), Some(&json(r#"["alpha"]"#)));
}

#[test]
fn object_arguments_are_used_as_given() {
    let fields = tool_call_arguments(
        &json(r#"{"vector_store_id":"vs_1"}"#),
        &ApiLimits::default(),
    )
    .unwrap();
    assert_eq!(fields.get("vector_store_id"), Some(&text("vs_1")));
}

#[test]
fn invalid_arguments_text_reads_as_empty() {
    let fields = tool_call_arguments(&text("{\"queries\": ["), &ApiLimits::default()).unwrap();
    assert!(fields.is_empty());
}

#[test]
fn arguments_that_are_not_a_json_object_are_refused() {
    let listed = tool_call_arguments(&text(r#"["alpha"]"#), &ApiLimits::default()).unwrap_err();
    assert!(matches!(listed, FileSearchLoopError::ArgumentsNotObject));
    let raw = tool_call_arguments(&Json::Null, &ApiLimits::default()).unwrap_err();
    assert!(matches!(raw, FileSearchLoopError::ArgumentsNotObject));
}

#[test]
fn arguments_beyond_the_json_limit_are_refused() {
    let limits = ApiLimits {
        max_json_bytes: 2,
        ..ApiLimits::default()
    };
    let error = tool_call_arguments(&text(&queries_arguments(&["alpha"])), &limits).unwrap_err();
    assert!(matches!(
        error,
        FileSearchLoopError::Limit(ApiLimitError::JsonTooLarge { .. })
    ));
}

#[test]
fn oversized_follow_up_input_is_refused() {
    let first = response(
        "resp_first",
        &[function_call_item(&queries_arguments(&["q"]), "call_1")],
    );
    let limits = ApiLimits {
        max_items: 2,
        ..ApiLimits::default()
    };
    let error = build_follow_up_input(&json(r#""question""#), &first.output, &[Json::Null], limits)
        .unwrap_err();
    assert!(matches!(
        error,
        FileSearchLoopError::Limit(ApiLimitError::TooManyItems { .. })
    ));
}

#[test]
fn decoder_rejects_a_document_that_is_not_an_object() {
    let error =
        decode_responses_file_search_args(br#"["alpha"]"#, &ApiLimits::default()).unwrap_err();
    assert!(matches!(error, DecodeError::NotObject));
    decode_responses_file_search_args(br#"{"queries":[]}"#, &ApiLimits::default()).unwrap();
}

#[test]
fn replays_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/responses_file_search_loop");
    let seeds: Vec<Vec<u8>> = fs::read_dir(corpus)
        .expect("the corpus folder")
        .map(|entry| fs::read(entry.expect("an entry").path()).expect("a seed file"))
        .collect();
    assert!(!seeds.is_empty());
    let decoded = seeds
        .iter()
        .filter(|bytes| {
            decode_responses_file_search_args(bytes.as_slice(), &ApiLimits::default()).is_ok()
        })
        .count();
    assert!(decoded >= 1);
}
