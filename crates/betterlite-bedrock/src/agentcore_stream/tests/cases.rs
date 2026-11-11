use betterlite_openai::stream::chunk::{Choice, Chunk};
use rstest::rstest;
use uuid::Uuid;

use crate::agentcore_stream::adapter::MAX_BODY_BYTES;
use crate::agentcore_stream::line::{LineError, MAX_LINE_BYTES, decode_agentcore_line};
use crate::agentcore_stream::tests::support::{
    MODEL, Outcome, Scenario, capture, ids, run, sorted_pairs,
};
use crate::agentcore_stream::transport::HandlerKind;
use crate::agentcore_stream::{
    AgentCoreStreamError, CHUNK_OBJECT, FIRST_RESPONSE, IdSource as _, PROVIDER, UuidV4,
};
use crate::bedrock_settings::ErrorHeaders;

#[rstest]
#[case::incremental_sse_stream(incremental_sse_stream)]
#[case::nonstream_json_fake_stream(nonstream_json_fake_stream)]
#[case::stream_usage(stream_usage)]
#[case::stream_terminal_event(stream_terminal_event)]
#[case::multiline_data_not_joined(multiline_data_not_joined)]
#[case::bad_json_skipped(bad_json_skipped)]
#[case::fresh_id_created_zero(fresh_id_created_zero)]
#[case::unterminated_line(unterminated_line)]
#[case::stream_http_request(stream_http_request)]
#[case::stream_http_errors(stream_http_errors)]
fn agentcore_stream_cases(#[case] case: fn()) {
    case();
}

fn both(scenario: &Scenario, values: &[Uuid]) -> [Outcome; 2] {
    [
        run(HandlerKind::Sync, scenario, values),
        run(HandlerKind::Async, scenario, values),
    ]
}

fn contents(chunks: &[Chunk]) -> Vec<&str> {
    chunks
        .iter()
        .filter_map(|chunk| chunk.choices.first())
        .filter_map(|choice| choice.delta.content.as_deref())
        .collect()
}

fn only_choice(chunk: &Chunk) -> &Choice {
    assert_eq!(chunk.choices.len(), 1);
    &chunk.choices[0]
}

fn assert_content_chunk(chunk: &Chunk) {
    assert_eq!(chunk.object, CHUNK_OBJECT);
    assert_eq!(chunk.created, 0);
    assert_eq!(chunk.model, MODEL);
    assert!(chunk.id.starts_with("chatcmpl-"));
    assert!(chunk.usage.is_none());
    let choice = only_choice(chunk);
    assert_eq!(choice.index, 0);
    assert_eq!(choice.finish_reason, None);
    assert_eq!(choice.delta.role.as_deref(), Some("assistant"));
    assert!(choice.delta.content.is_some());
}

fn assert_stop_chunk(chunk: &Chunk, usage: Option<(u64, u64, u64)>) {
    assert_eq!(chunk.object, CHUNK_OBJECT);
    assert_eq!(chunk.created, 0);
    assert_eq!(chunk.model, MODEL);
    let choice = only_choice(chunk);
    assert_eq!(choice.index, 0);
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    assert!(choice.delta.content.is_none());
    assert!(choice.delta.role.is_none());
    match (usage, &chunk.usage) {
        (None, None) => {}
        (Some((prompt, completion, total)), Some(found)) => {
            assert_eq!(found.prompt_tokens, prompt);
            assert_eq!(found.completion_tokens, completion);
            assert_eq!(found.total_tokens, total);
        }
        (expected, found) => panic!("usage {found:?} does not match {expected:?}"),
    }
}

fn line_error(error: AgentCoreStreamError) -> LineError {
    let AgentCoreStreamError::Line(source) = error else {
        panic!("expected a line error");
    };
    source
}

fn incremental_sse_stream() {
    let reads = [
        "data: {\"event\":{\"contentBlockDelta\":{\"del",
        "ta\":{\"text\":\"he\"}}}}\n",
        "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"llo\"}}}}\n\n",
        "\n",
        "event: ping\n",
        "data:\n",
        "data: 42\n",
        "data: [1]\n",
        "data: null\n",
        "data: {\"event\":5}\n",
        "  data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"!\"}}}}\r\n",
        "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{}}}}\n",
        "data: {\"event\":{\"contentBlockDelta\":{}}}\n",
        "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"\"}}}}\n",
    ];
    for outcome in both(&Scenario::sse(&reads), &ids(8)) {
        let chunks = outcome.emitted.unwrap();
        assert_eq!(contents(&chunks), ["he", "llo", "!"]);
        for chunk in &chunks {
            assert_content_chunk(chunk);
        }
    }
}

fn nonstream_json_fake_stream() {
    for (body, content) in [
        (r#"{"result":{"content":[{"text":"hello"}]}}"#, "hello"),
        (
            r#"{"jsonrpc":"2.0","result":{"message":{"parts":[{"kind":"text","text":"a2a"}]}}}"#,
            "a2a",
        ),
        (r#"{"z": 1, "a": 2}"#, r#"{"z": 1, "a": 2}"#),
        (r#"{"result":{"content":[]}}"#, ""),
        (
            r#"{"result":{"content":[{"text":"x"}]},"usage":{"inputTokens":9}}"#,
            "x",
        ),
    ] {
        for outcome in both(&Scenario::json(body), &ids(2)) {
            let chunks = outcome.emitted.unwrap();
            assert_eq!(chunks.len(), 2);
            assert_content_chunk(&chunks[0]);
            assert_eq!(contents(&chunks), [content]);
            assert_stop_chunk(&chunks[1], None);
            assert_eq!(outcome.handler, "default");
            assert_eq!(outcome.provider, Some(PROVIDER));
        }
    }
    let mut upper = Scenario::json(r#"{"result":{"content":[{"text":"upper"}]}}"#);
    upper.response_headers = vec![("CONTENT-TYPE".to_owned(), "Application/JSON".to_owned())];
    for outcome in both(&upper, &ids(2)) {
        assert_eq!(contents(&outcome.emitted.unwrap()), ["upper"]);
    }
}

fn stream_usage() {
    let line = "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"hi\"}},\"metadata\":{\"usage\":{\"inputTokens\":1,\"outputTokens\":2,\"totalTokens\":3}}}}\n";
    for outcome in both(&Scenario::sse(&[line]), &ids(2)) {
        let chunks = outcome.emitted.unwrap();
        assert_eq!(contents(&chunks), ["hi"]);
        assert_stop_chunk(&chunks[1], Some((1, 2, 3)));
    }
    for outcome in both(
        &Scenario::sse(&[r#"data: {"event":{"metadata":{"usage":{}}}}"#]),
        &ids(1),
    ) {
        let chunks = outcome.emitted.unwrap();
        assert_eq!(chunks.len(), 1);
        assert_stop_chunk(&chunks[0], Some((0, 0, 0)));
    }
    let partial = r#"data: {"event":{"metadata":{"usage":{"outputTokens":9}}}}"#;
    for outcome in both(&Scenario::sse(&[partial]), &ids(1)) {
        assert_stop_chunk(&outcome.emitted.unwrap()[0], Some((0, 9, 0)));
    }
    let independent = r#"data: {"event":{"metadata":{"usage":{"totalTokens":5}}}}"#;
    for outcome in both(&Scenario::sse(&[independent]), &ids(1)) {
        assert_stop_chunk(&outcome.emitted.unwrap()[0], Some((0, 0, 5)));
    }
    let null_field =
        r#"data: {"event":{"metadata":{"usage":{"inputTokens":null,"outputTokens":2}}}}"#;
    for outcome in both(&Scenario::sse(&[null_field]), &ids(1)) {
        assert_stop_chunk(&outcome.emitted.unwrap()[0], Some((0, 2, 0)));
    }
    let combined = r#"data: {"event":{"contentBlockDelta":{"delta":{"text":"x"}},"metadata":{"usage":{"inputTokens":4}}},"message":{}}"#;
    for outcome in both(&Scenario::sse(&[combined]), &ids(3)) {
        let chunks = outcome.emitted.unwrap();
        assert_eq!(contents(&chunks), ["x"]);
        assert_stop_chunk(&chunks[1], Some((4, 0, 0)));
        assert_stop_chunk(&chunks[2], None);
    }
    stream_usage_failures();
}

fn usage_failure(read: &str) {
    for outcome in both(&Scenario::sse(&[read]), &ids(1)) {
        assert!(matches!(
            line_error(outcome.emitted.unwrap_err()),
            LineError::UsageNotObject
                | LineError::UsageNotInteger { .. }
                | LineError::UsageNegative { .. }
                | LineError::MetadataNotObject
        ));
    }
}

fn stream_usage_failures() {
    for read in [
        r#"data: {"event":{"metadata":{"usage":null}}}"#,
        r#"data: {"event":{"metadata":{"usage":5}}}"#,
        r#"data: {"event":{"metadata":{"usage":{"inputTokens":"1"}}}}"#,
        r#"data: {"event":{"metadata":5}}"#,
    ] {
        usage_failure(read);
    }
    let negative = r#"data: {"event":{"metadata":{"usage":{"inputTokens":-1}}}}"#;
    for outcome in both(&Scenario::sse(&[negative]), &ids(1)) {
        assert!(matches!(
            line_error(outcome.emitted.unwrap_err()),
            LineError::UsageNegative {
                field: "inputTokens"
            }
        ));
    }
}

fn stream_terminal_event() {
    for outcome in both(&Scenario::sse(&[r#"data: {"message":{}}"#]), &ids(1)) {
        let chunks = outcome.emitted.unwrap();
        assert_eq!(chunks.len(), 1);
        assert_stop_chunk(&chunks[0], None);
    }
    let with_content = r#"data: {"message":{"content":[{"text":"ignored"}]}}"#;
    for outcome in both(&Scenario::sse(&[with_content]), &ids(1)) {
        let chunks = outcome.emitted.unwrap();
        assert!(contents(&chunks).is_empty());
        assert_stop_chunk(&chunks[0], None);
    }
    let continued = [
        r#"data: {"message":{}}"#,
        r#"data: {"event":{"contentBlockDelta":{"delta":{"text":"after"}}}}"#,
    ];
    for outcome in both(&Scenario::sse(&continued), &ids(2)) {
        let chunks = outcome.emitted.unwrap();
        assert_eq!(contents(&chunks), ["after"]);
        assert_stop_chunk(&chunks[0], None);
        assert_content_chunk(&chunks[1]);
    }
    let repeated = [
        r#"data: {"message":{}}"#,
        r#"data: {"message":{}}"#,
        r#"data: {"event":{"metadata":{"usage":{}}}}"#,
    ];
    for outcome in both(&Scenario::sse(&repeated), &ids(3)) {
        let chunks = outcome.emitted.unwrap();
        assert_eq!(chunks.len(), 3);
        assert_stop_chunk(&chunks[0], None);
        assert_stop_chunk(&chunks[1], None);
        assert_stop_chunk(&chunks[2], Some((0, 0, 0)));
    }
    let done = ["data: [DONE]\n", "data: {\"message\":5}\n"];
    for outcome in both(&Scenario::sse(&done), &ids(1)) {
        assert!(outcome.emitted.unwrap().is_empty());
    }
}

fn multiline_data_not_joined() {
    let separate = [
        r#"data: {"event":{"contentBlockDelta":{"delta":{"text":"one"}}}}"#,
        r#"data: {"event":{"contentBlockDelta":{"delta":{"text":"two"}}}}"#,
    ];
    for outcome in both(&Scenario::sse(&separate), &ids(2)) {
        assert_eq!(contents(&outcome.emitted.unwrap()), ["one", "two"]);
    }
    let broken = [
        "data: {\"event\":",
        "data: {\"contentBlockDelta\":{\"delta\":{\"text\":\"joined\"}}}}\n",
    ];
    for outcome in both(&Scenario::sse(&broken), &ids(1)) {
        assert!(outcome.emitted.unwrap().is_empty());
    }
    let crlf = ["  data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"crlf\"}}}}\r\n"];
    for outcome in both(&Scenario::sse(&crlf), &ids(1)) {
        assert_eq!(contents(&outcome.emitted.unwrap()), ["crlf"]);
    }
    let split = [
        "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"ke",
        "pt\"}}}}\n",
    ];
    for outcome in both(&Scenario::sse(&split), &ids(1)) {
        assert_eq!(contents(&outcome.emitted.unwrap()), ["kept"]);
    }
}

fn bad_json_skipped() {
    let (logs, _guard) = capture();
    let reads = [
        "data: {not json}\n",
        "data: [DONE]\n",
        r#"data: {"event":{"contentBlockDelta":{"delta":{"text":"ok"}}}}"#,
    ];
    for outcome in both(&Scenario::sse(&reads), &ids(1)) {
        assert_eq!(contents(&outcome.emitted.unwrap()), ["ok"]);
    }
    let text = logs.text();
    assert!(text.contains("Skipping non-JSON SSE line"));
    assert!(!text.contains("not json"));
    for (read, expected) in [
        (r#"data: {"event":{"contentBlockDelta":5}}"#, true),
        (
            r#"data: {"event":{"contentBlockDelta":{"delta":5}}}"#,
            false,
        ),
    ] {
        for outcome in both(&Scenario::sse(&[read]), &ids(1)) {
            let error = line_error(outcome.emitted.unwrap_err());
            assert_eq!(matches!(error, LineError::DeltaEventNotObject), expected);
            assert_eq!(matches!(error, LineError::DeltaNotObject), !expected);
        }
    }
    let bad_text = r#"data: {"event":{"contentBlockDelta":{"delta":{"text":5}}}}"#;
    for outcome in both(&Scenario::sse(&[bad_text]), &ids(1)) {
        assert!(matches!(
            line_error(outcome.emitted.unwrap_err()),
            LineError::DeltaTextNotString
        ));
    }
}

fn fresh_id_created_zero() {
    let sequence = ids(6);
    let reads = [
        r#"data: {"event":{"contentBlockDelta":{"delta":{"text":"a"}},"metadata":{"usage":{}}}}"#,
        r#"data: {"message":{}}"#,
    ];
    for outcome in both(&Scenario::sse(&reads), &sequence) {
        let chunks = outcome.emitted.unwrap();
        assert_eq!(chunks.len(), 3);
        let found: Vec<String> = chunks.iter().map(|chunk| chunk.id.clone()).collect();
        let expected: Vec<String> = sequence
            .iter()
            .take(3)
            .map(|id| format!("chatcmpl-{id}"))
            .collect();
        assert_eq!(found, expected);
        assert_eq!(outcome.remaining_ids, 3);
        assert_eq!(outcome.provider, Some(PROVIDER));
        for chunk in &chunks {
            assert_eq!(chunk.created, 0);
            assert_eq!(chunk.object, CHUNK_OBJECT);
            assert_eq!(chunk.model, MODEL);
            assert_eq!(only_choice(chunk).index, 0);
        }
    }
    let json = Scenario::json(r#"{"result":{"content":[{"text":"x"}]}}"#);
    for outcome in both(&json, &ids(2)) {
        let chunks = outcome.emitted.unwrap();
        assert_eq!(chunks.len(), 2);
        assert_ne!(chunks[0].id, chunks[1].id);
        assert_eq!(outcome.remaining_ids, 0);
    }
}

fn unterminated_line() {
    let unterminated =
        ["data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"lost\"}}}}"];
    for outcome in both(&Scenario::sse(&unterminated), &ids(1)) {
        assert!(outcome.emitted.unwrap().is_empty());
    }
    for outcome in both(&Scenario::sse(&["", "\n"]), &ids(1)) {
        assert!(outcome.emitted.unwrap().is_empty());
    }
    let delayed = [
        "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"kept\"}}",
        "}}\n",
    ];
    for outcome in both(&Scenario::sse(&delayed), &ids(1)) {
        assert_eq!(contents(&outcome.emitted.unwrap()), ["kept"]);
    }
    let trailing = [
        "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"first\"}}}}\n",
        "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"residual\"}}}}",
    ];
    for outcome in both(&Scenario::sse(&trailing), &ids(1)) {
        let chunks = outcome.emitted.unwrap();
        assert_eq!(contents(&chunks), ["first"]);
        assert_eq!(outcome.remaining_ids, 0);
    }
}

fn request_scenario() -> Scenario {
    let mut scenario = Scenario::sse(&[
        "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"sent\"}}}}\n",
    ]);
    scenario.request_headers = vec![
        (
            "accept".to_owned(),
            "application/json, text/event-stream".to_owned(),
        ),
        ("x-caller".to_owned(), "yes".to_owned()),
    ];
    scenario.data = serde_json::from_str(r#"{"prompt":"hi"}"#).unwrap();
    scenario.messages = serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).unwrap();
    scenario
}

fn assert_request(outcome: &Outcome, scenario: &Scenario) {
    assert_eq!(outcome.posts.len(), 1);
    let post = &outcome.posts[0];
    assert_eq!(post.url, scenario.api_base);
    assert!(post.stream);
    assert_eq!(post.body, serde_json::to_vec(&scenario.data).unwrap());
    assert_eq!(post.headers, sorted_pairs(&scenario.request_headers));
    assert_eq!(outcome.post_calls.len(), 1);
    let call = &outcome.post_calls[0];
    assert_eq!(call.input, scenario.messages);
    assert_eq!(call.api_key, "");
    assert_eq!(call.original_response, FIRST_RESPONSE);
    assert_eq!(call.complete_input_dict, scenario.data);
}

fn stream_http_request() {
    let mut scenario = request_scenario();
    scenario.supplied = Some(HandlerKind::Sync);
    let outcomes = both(&scenario, &ids(1));
    assert_eq!(outcomes[0].handler, "supplied");
    assert_eq!(outcomes[1].handler, "default");
    for outcome in &outcomes {
        assert_request(outcome, &scenario);
        assert_eq!(contents(outcome.emitted.as_ref().unwrap()), ["sent"]);
        assert_eq!(outcome.provider, Some(PROVIDER));
    }
    let mut signed = request_scenario();
    signed.signed_json_body = Some(b"{\"signed\":true}".to_vec());
    let signed_outcome = run(HandlerKind::Sync, &signed, &ids(1));
    assert_eq!(signed_outcome.posts[0].body, b"{\"signed\":true}".to_vec());
    let mut fallback = request_scenario();
    fallback.signed_json_body = Some(Vec::new());
    let fallback_outcome = run(HandlerKind::Async, &fallback, &ids(1));
    assert_eq!(
        fallback_outcome.posts[0].body,
        serde_json::to_vec(&fallback.data).unwrap()
    );
}

fn provider_fields(error: &AgentCoreStreamError) -> (u16, Vec<(String, String)>, u16) {
    let AgentCoreStreamError::Provider { status, cause } = error else {
        panic!("expected a provider error");
    };
    let headers = match &cause.cause.headers {
        Some(ErrorHeaders::Http(pairs)) => pairs.clone(),
        Some(ErrorHeaders::Mapping(_)) | None => Vec::new(),
    };
    (*status, headers, cause.cause.error.response.status_code)
}

fn non_200_error() {
    let mut scenario = request_scenario();
    scenario.status = 500;
    scenario.reads = vec!["boom".to_owned()];
    scenario.response_headers = vec![
        ("content-type".to_owned(), "text/plain".to_owned()),
        ("x-request-id".to_owned(), "raw".to_owned()),
    ];
    for outcome in both(&scenario, &ids(1)) {
        let (status, headers, response_status) = provider_fields(&outcome.emitted.unwrap_err());
        assert_eq!(status, 500);
        assert_eq!(response_status, 500);
        assert_eq!(headers, sorted_pairs(&scenario.response_headers));
        assert_eq!(outcome.posts.len(), 1);
        assert!(outcome.post_calls.is_empty());
    }
}

fn json_error(scenario: &Scenario, status: u16) {
    for outcome in both(scenario, &ids(2)) {
        let (found, headers, response_status) = provider_fields(&outcome.emitted.unwrap_err());
        assert_eq!(found, status);
        assert_eq!(response_status, status);
        assert_eq!(
            headers,
            vec![("content-type".to_owned(), "application/json".to_owned())]
        );
    }
}

fn projection_error() {
    let projection = Scenario::json(r#"{"result":{"content":[{"text":5}]}}"#);
    for outcome in both(&projection, &ids(2)) {
        assert!(matches!(
            outcome.emitted.unwrap_err(),
            AgentCoreStreamError::Reply(_)
        ));
    }
}

fn stream_http_errors() {
    non_200_error();
    let mut early = Scenario::json(r#"{"result":{"content":[{"text":"kept"}]}}"#);
    early.status = 201;
    json_error(&early, 201);
    json_error(&Scenario::json(r#"{"result":"#), 200);
    projection_error();
}

#[test]
fn agentcore_stream_bounded_inputs() {
    let long_read = "x".repeat(MAX_LINE_BYTES.saturating_add(1));
    let mut buffer = Vec::new();
    let error = decode_agentcore_line(&mut buffer, long_read.as_bytes()).unwrap_err();
    assert!(matches!(error, LineError::TooLarge { .. }));
    let mut exact = Vec::new();
    let boundary = "x".repeat(MAX_LINE_BYTES);
    assert!(
        decode_agentcore_line(&mut exact, boundary.as_bytes())
            .unwrap()
            .is_empty()
    );
    let mut complete = String::from("data: ");
    complete.push_str(&"x".repeat(MAX_LINE_BYTES));
    complete.push('\n');
    let mut complete_buffer = Vec::new();
    assert!(
        decode_agentcore_line(&mut complete_buffer, complete.as_bytes())
            .unwrap()
            .is_empty()
    );
}

#[rstest]
fn agentcore_stream_body_cap(#[values(HandlerKind::Sync, HandlerKind::Async)] kind: HandlerKind) {
    let padding = " ".repeat(MAX_BODY_BYTES.saturating_sub(2));
    let at_limit = Scenario::json(&format!("{padding}{{}}"));
    assert_eq!(run(kind, &at_limit, &ids(2)).emitted.unwrap().len(), 2);
    let mut over = Scenario::json("{}");
    over.reads = vec!["x".repeat(MAX_BODY_BYTES.saturating_add(1))];
    let AgentCoreStreamError::Provider { cause, .. } =
        run(kind, &over, &ids(2)).emitted.unwrap_err()
    else {
        panic!("a body over the cap fails as a provider error");
    };
    assert!(cause.cause.error.message.contains("larger than"));
}

#[test]
fn uuid_v4_source_returns_a_random_id() {
    let source = UuidV4;
    let first = source.next_id();
    assert_ne!(first, Uuid::nil());
    assert_ne!(first, source.next_id());
}
