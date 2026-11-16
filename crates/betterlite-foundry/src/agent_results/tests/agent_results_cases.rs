#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use secrecy::SecretString;

use crate::agent_requests::prepare::{Headers, PreparedAgentCall};
use crate::agent_results::messages::{ExtractedContent, extract_content, transform_annotations};
use crate::agent_results::reply::{
    AgentChoice, AgentReply, ReplyInput, ReplyMessage, Usage, build_model_response,
};
use crate::agent_results::request::{
    StreamPayload, ThreadMessage, ThreadPayload, build_stream_request,
};
use crate::agent_results::stream::{AgentChunk, STATUS_CREATED, STATUS_OK, open_stream};

use super::support::{
    FakeCounter, Recorded, block_on, body_stream, chunks_of, delta_chunk, messages, object, setup,
    shown_headers, split_bytes, stream_for, stream_for_chunks, terminal_chunk, whole,
    with_warnings,
};

fn assistant_first_text() {
    let text = r#"{
        "data": [
            {"role": "user", "content": [{"type": "text", "text": {"value": "ignored"}}]},
            {"role": "assistant", "content": [
                {"type": "image_url"},
                {"type": "text", "text": {"value": "", "annotations": [{"type": "other"}]}},
                {"type": "text", "text": {"value": "later"}}
            ]},
            {"role": "assistant", "content": [{"type": "text", "text": {"value": "third"}}]}
        ]
    }"#;
    let extracted = extract_content(&messages(text));
    assert_eq!(extracted.content, "");
    assert_eq!(
        extracted.annotations,
        Some(vec![object(r#"{"type": "other"}"#)])
    );

    let skipped = r#"{"data": [{"role": "assistant", "content": []},
        {"role": "assistant", "content": [{"type": "text", "text": {"value": "found"}}]}]}"#;
    assert_eq!(extract_content(&messages(skipped)).content, "found");

    let no_value = r#"{"data": [{"role": "assistant", "content": [{"type": "text"}]}]}"#;
    let extracted = extract_content(&messages(no_value));
    assert_eq!(extracted.content, "");
    assert_eq!(extracted.annotations, None);

    let null_annotations = r#"{"data": [{"role": "assistant", "content": [
        {"type": "text", "text": {"value": "hi", "annotations": null}}]}]}"#;
    let extracted = extract_content(&messages(null_annotations));
    assert_eq!(extracted.content, "hi");
    assert_eq!(extracted.annotations, None);

    let empty_annotations = r#"{"data": [{"role": "assistant", "content": [
        {"type": "text", "text": {"value": "hi", "annotations": []}}]}]}"#;
    assert_eq!(
        extract_content(&messages(empty_annotations)).annotations,
        None
    );

    assert_eq!(
        extract_content(&messages("{}")),
        ExtractedContent::default()
    );
}

fn citation_index_relocation() {
    assert_eq!(transform_annotations(None), None);
    assert_eq!(transform_annotations(Some(&[])), None);

    let raw = vec![
        object(
            r#"{"type": "url_citation", "text": "[1]", "start_index": 10, "end_index": 13,
                "url_citation": {"url": "https://a", "title": "A"}}"#,
        ),
        object(
            r#"{"type": "url_citation", "start_index": 7,
                "url_citation": {"url": "https://b", "start_index": null}}"#,
        ),
        object(r#"{"type": "url_citation", "end_index": 9}"#),
        object(r#"{"type": "file_citation", "file_id": "f1"}"#),
    ];
    let transformed = transform_annotations(Some(&raw)).expect("transformed annotations");
    assert_eq!(
        transformed,
        vec![
            object(
                r#"{"type": "url_citation",
                    "url_citation": {"url": "https://a", "title": "A", "start_index": 10, "end_index": 13}}"#,
            ),
            object(
                r#"{"type": "url_citation", "url_citation": {"url": "https://b", "start_index": null}}"#,
            ),
            object(r#"{"type": "url_citation", "url_citation": {"end_index": 9}}"#),
            object(r#"{"type": "file_citation", "file_id": "f1"}"#),
        ]
    );

    // the citation object LiteLLM would unpack with ** is tolerated as empty here
    let tolerant = vec![object(
        r#"{"type": "url_citation", "url_citation": "not-an-object", "start_index": 3}"#,
    )];
    assert_eq!(
        transform_annotations(Some(&tolerant)),
        Some(vec![object(
            r#"{"type": "url_citation", "url_citation": {"start_index": 3}}"#,
        )])
    );
}

fn thread_id_metadata() {
    block_on(async {
        let counter = FakeCounter::counting(11, 5);
        let existing = AgentReply {
            hidden_params: Some(object(r#"{"request-id": "r1", "thread_id": "old"}"#)),
            usage: Some(Usage {
                prompt_tokens: 1,
                completion_tokens: 2,
                total_tokens: 3,
            }),
            ..AgentReply::default()
        };
        let input = ReplyInput {
            model: "azure_ai/agents/a",
            content: "hello",
            thread_id: "thread-9",
            messages: &[],
            annotations: Some(Vec::new()),
        };
        let reply = build_model_response(existing, &input, &counter).await;
        assert_eq!(reply.model.as_deref(), Some("azure_ai/agents/a"));
        assert_eq!(
            reply.choices,
            vec![AgentChoice {
                index: 0,
                finish_reason: String::from("stop"),
                message: ReplyMessage {
                    role: String::from("assistant"),
                    content: String::from("hello"),
                    annotations: None,
                },
            }]
        );
        assert_eq!(
            reply.hidden_params,
            Some(object(r#"{"request-id": "r1", "thread_id": "thread-9"}"#))
        );
        assert_eq!(
            reply.usage,
            Some(Usage {
                prompt_tokens: 11,
                completion_tokens: 5,
                total_tokens: 16,
            })
        );

        let annotations = vec![object(r#"{"type": "file_citation", "file_id": "f1"}"#)];
        let with_annotations = ReplyInput {
            annotations: Some(annotations.clone()),
            ..input
        };
        let annotated =
            build_model_response(AgentReply::default(), &with_annotations, &counter).await;
        assert_eq!(
            annotated
                .choices
                .first()
                .and_then(|choice| choice.message.annotations.as_ref()),
            Some(&annotations)
        );
        assert_eq!(
            annotated.hidden_params,
            Some(object(r#"{"thread_id": "thread-9"}"#))
        );
    });
}

fn token_count_warning() {
    block_on(async {
        let counter = FakeCounter::counting(7, 3);
        let messages = vec![object(r#"{"role": "user", "content": "hi"}"#)];
        let input = ReplyInput {
            model: "azure_ai/agents/a",
            content: "hello",
            thread_id: "thread-9",
            messages: &messages,
            annotations: None,
        };
        let (counted, counted_warnings) = with_warnings(build_model_response(
            AgentReply::default(),
            &input,
            &counter,
        ))
        .await;
        assert!(counted_warnings.is_empty());
        assert_eq!(
            counter.records(),
            vec![
                Recorded {
                    model: String::from("gpt-3.5-turbo"),
                    messages: Some(messages.clone()),
                    text: None,
                    count_response_tokens: false,
                },
                Recorded {
                    model: String::from("gpt-3.5-turbo"),
                    messages: None,
                    text: Some(String::from("hello")),
                    count_response_tokens: true,
                },
            ]
        );
        assert_eq!(
            counted.usage,
            Some(Usage {
                prompt_tokens: 7,
                completion_tokens: 3,
                total_tokens: 10,
            })
        );

        let existing = Usage {
            prompt_tokens: 4,
            completion_tokens: 6,
            total_tokens: 10,
        };
        let failing = FakeCounter::failing();
        let (failed, failed_warnings) = with_warnings(build_model_response(
            AgentReply {
                usage: Some(existing),
                ..AgentReply::default()
            },
            &input,
            &failing,
        ))
        .await;
        assert_eq!(failed.usage, Some(existing));
        assert_eq!(
            failed_warnings,
            vec![String::from(
                "Failed to calculate token usage: either text or messages must be provided"
            )]
        );
        assert_eq!(failing.records().len(), 1);

        let (unreported, _warnings) = with_warnings(build_model_response(
            AgentReply::default(),
            &input,
            &failing,
        ))
        .await;
        assert_eq!(unreported.usage, None);

        let completion_failing = FakeCounter::failing_completion();
        let (kept, kept_warnings) = with_warnings(build_model_response(
            AgentReply {
                usage: Some(existing),
                ..AgentReply::default()
            },
            &input,
            &completion_failing,
        ))
        .await;
        assert_eq!(kept.usage, Some(existing));
        assert_eq!(kept_warnings.len(), 1);
        assert_eq!(completion_failing.records().len(), 2);
    });
}

fn message_delta_stream() {
    block_on(async {
        let spaced = concat!(
            "  event: thread.message.delta  \n\n",
            "data:  {\"delta\":{\"content\":[{\"type\":\"text\",\"text\":{\"value\":\"Hel\"}}]}}  \n\n",
            "data: [DONE]\n\n",
        );
        let chunks = chunks_of(stream_for(spaced).await).await;
        assert_eq!(chunks, vec![delta_chunk("Hel"), terminal_chunk(None, None)]);

        let compact = concat!(
            "event: thread.message.delta\n\n",
            "data: {\"delta\":{\"content\":[{\"type\":\"text\",\"text\":{\"value\":\"a\"}}]}}\n",
            "data: {\"delta\":{\"content\":[{\"type\":\"text\",\"text\":{\"value\":\"b\"}}]}}\n\n",
            "data: not-json\n\n",
            "data: {\"delta\":{\"content\":[{\"type\":\"image_url\"},{\"type\":\"text\",\n",
            "  \"text\":{\"value\":\"\"}}, {\"type\": \"text\"}]}}\n\n",
            "event: thread.message.completed\n",
            "data: {\"content\": []}\n\n",
        );
        let chunks = chunks_of(stream_for(compact).await).await;
        assert_eq!(chunks, vec![delta_chunk("a"), delta_chunk("b")]);
    });
}

fn run_terminal_stream() {
    block_on(async {
        let citation =
            r#"{"type": "url_citation", "url_citation": {"url": "https://a"}, "start_index": 1}"#;
        let template = concat!(
            "event: thread.created\ndata: {\"id\": \"thread_1\"}\n\n",
            "event: thread.message.delta\n",
            "data: {\"delta\":{\"content\":[{\"type\":\"text\",\"text\":{\"value\":\"Hi\"}}]}}\n\n",
            "event: thread.message.completed\n",
            "data: {\"content\":[{\"type\":\"text\",\"text\":{\"value\":\"\",\"annotations\":[CITE]}}]}\n\n",
            "event: run.failed\ndata: {\"error\": \"boom\"}\n\n",
            "event: thread.message.completed\n",
            "data: {\"content\":[{\"type\":\"text\",\"text\":{\"annotations\":[CITE]}}]}\n\n",
            "data: [DONE]\n\n",
            "data: {\"delta\":{\"content\":[{\"type\":\"text\",\"text\":{\"value\":\"after\"}}]}}\n\n",
        );
        let full_body = template.replace("CITE", citation);
        let chunks = chunks_of(stream_for(&full_body).await).await;
        let annotations = Some(vec![
            object(
                r#"{"type": "url_citation", "url_citation": {"url": "https://a", "start_index": 1}}"#,
            ),
            object(
                r#"{"type": "url_citation", "url_citation": {"url": "https://a", "start_index": 1}}"#,
            ),
        ]);
        assert_eq!(
            chunks,
            vec![
                AgentChunk {
                    thread_id: Some(String::from("thread_1")),
                    ..delta_chunk("Hi")
                },
                terminal_chunk(annotations, Some(String::from("thread_1"))),
            ]
        );

        let unterminated = concat!(
            "event: thread.message.delta\n",
            "data: {\"delta\":{\"content\":[{\"type\":\"text\",\"text\":{\"value\":\"only\"}}]}}",
        );
        assert_eq!(
            chunks_of(stream_for(unterminated).await).await,
            vec![delta_chunk("only")]
        );

        let empty_delta =
            "event: thread.message.delta\ndata: {\"delta\":{\"content\":[]}}\n\ndata: [DONE]";
        assert_eq!(
            chunks_of(stream_for(empty_delta).await).await,
            vec![terminal_chunk(None, None)]
        );

        let thread_events = concat!(
            "event: thread.created\ndata: {\"id\": \"thread_2\"}\n\n",
            "event: thread.created\ndata: {\"note\": \"no id\"}\n\n",
            "event: thread.message.delta\n",
            "data: {\"delta\":{\"content\":[{\"type\":\"text\",\"text\":{\"value\":\"c\"}}]}}\n\n",
            "event: thread.created\ndata: {\"id\": null}\n\n",
            "event: thread.message.delta\n",
            "data: {\"delta\":{\"content\":[{\"type\":\"text\",\"text\":{\"value\":\"d\"}}]}}\n\n",
        );
        assert_eq!(
            chunks_of(stream_for(thread_events).await).await,
            vec![
                AgentChunk {
                    thread_id: Some(String::from("thread_2")),
                    ..delta_chunk("c")
                },
                delta_chunk("d"),
            ]
        );
    });
}

fn split_sse_event() {
    block_on(async {
        let crlf = concat!(
            "event: thread.message.delta\r\n\r\n",
            "data: {\"delta\":{\"content\":[{\"type\":\"text\",\"text\":{\"value\":\"héllo 世界\"}}]}}\r\n\r\n",
        );
        let split_at = vec![
            crlf.find("\r\n").expect("a CRLF").saturating_add(1),
            crlf.find('é').expect("an accent").saturating_add(1),
            crlf.find('世').expect("a wide character").saturating_add(1),
        ];
        let split = split_bytes(crlf.as_bytes(), &split_at);
        let chunks = chunks_of(stream_for_chunks(split).await).await;
        assert_eq!(chunks, chunks_of(stream_for(crlf).await).await);
        assert_eq!(chunks, vec![delta_chunk("héllo 世界")]);

        let repeated = concat!(
            "event: thread.message.delta\n\n",
            "data: {\"delta\":{\"content\":[{\"type\":\"text\",\"text\":{\"value\":\"x\"}}]}}\n",
            "data: {\"delta\":{\"content\":[{\"type\":\"text\",\"text\":{\"value\":\"y\"}}]}}\n\n",
        );
        assert_eq!(
            chunks_of(stream_for(repeated).await).await,
            vec![delta_chunk("x"), delta_chunk("y")]
        );
    });
}

fn stream_thread_payload() {
    let headers: Headers = BTreeMap::from([
        (
            String::from("Authorization"),
            SecretString::from("Bearer secret"),
        ),
        (
            String::from("Content-Type"),
            SecretString::from("application/json"),
        ),
    ]);
    let prepared = PreparedAgentCall {
        headers: headers.clone(),
        api_version: String::from("2025-05-01"),
        agent_id: String::from("asst_1"),
        thread_id: None,
        api_base: String::from("https://base.example/api/projects/p"),
    };
    let thread_input = vec![
        object(r#"{"role": "user", "content": "hi"}"#),
        object(r#"{"role": "system", "content": "sys"}"#),
        object(r#"{"role": "assistant", "content": "skip"}"#),
        object(r#"{"content": "no role"}"#),
        object(r#"{"role": "tool", "content": "skip"}"#),
        object(r#"{"role": "user"}"#),
    ];
    let optional_params = object(r#"{"instructions": "be brief"}"#);
    let litellm_params = object(r#"{"ssl_verify": false}"#);
    let fresh = build_stream_request(&prepared, &thread_input, &optional_params, &litellm_params)
        .expect("a stream request");
    assert_eq!(
        fresh.url,
        "https://base.example/api/projects/p/threads/runs?api-version=2025-05-01"
    );
    assert_eq!(shown_headers(&fresh.headers), shown_headers(&headers));
    assert_eq!(fresh.ssl_verify, Json::Bool(false));
    assert_eq!(
        fresh.payload,
        StreamPayload {
            assistant_id: String::from("asst_1"),
            stream: true,
            thread: Some(ThreadPayload {
                messages: vec![
                    ThreadMessage {
                        role: String::from("user"),
                        content: Json::Text(String::from("hi")),
                    },
                    ThreadMessage {
                        role: String::from("user"),
                        content: Json::Text(String::from("sys")),
                    },
                    ThreadMessage {
                        role: String::from("user"),
                        content: Json::Text(String::new()),
                    },
                ],
            }),
            instructions: Some(Json::Text(String::from("be brief"))),
        }
    );

    let existing = PreparedAgentCall {
        thread_id: Some(String::from("thread-7")),
        ..prepared
    };
    let existing_request = build_stream_request(
        &existing,
        &thread_input,
        &object("{}"),
        &object(r#"{"ssl_verify": null}"#),
    )
    .expect("a stream request");
    assert!(existing_request.payload.thread.is_none());
    assert!(existing_request.payload.instructions.is_none());
    assert_eq!(existing_request.ssl_verify, Json::Null);
    assert_eq!(
        serde_json::to_string(&existing_request.payload).expect("the payload"),
        r#"{"assistant_id":"asst_1","stream":true}"#
    );

    let empty = PreparedAgentCall {
        thread_id: Some(String::new()),
        ..prepared
    };
    let empty_thread_request = build_stream_request(
        &empty,
        &thread_input,
        &object(r#"{"instructions": null}"#),
        &object("{}"),
    )
    .expect("a stream request");
    assert_eq!(
        empty_thread_request
            .payload
            .thread
            .map(|thread| thread.messages.len()),
        Some(3)
    );
    assert_eq!(empty_thread_request.payload.instructions, Some(Json::Null));
    assert_eq!(empty_thread_request.ssl_verify, Json::Null);
    assert_eq!(
        serde_json::to_string(&empty_thread_request.payload).expect("the payload"),
        concat!(
            r#"{"assistant_id":"asst_1","stream":true,"thread":{"messages":["#,
            r#"{"role":"user","content":"hi"},{"role":"user","content":"sys"},"#,
            r#"{"role":"user","content":""}]},"instructions":null}"#,
        )
    );
}

fn stream_status_failure() {
    block_on(async {
        let ok_stream = open_stream(setup(STATUS_OK), whole("data: [DONE]\n\n"))
            .await
            .expect("a stream");
        assert_eq!(chunks_of(ok_stream).await, vec![terminal_chunk(None, None)]);
        let created_stream = open_stream(setup(STATUS_CREATED), whole(""))
            .await
            .expect("a stream");
        assert!(chunks_of(created_stream).await.is_empty());

        for status in [400_u16, 500] {
            let failure = open_stream(setup(status), whole("bad request"))
                .await
                .err()
                .expect("a failure");
            assert_eq!(failure.status(), Some(status));
            assert_eq!(failure.to_string(), "Streaming request failed: bad request");
        }

        let binary_failure = open_stream(setup(502), body_stream(vec![vec![0xff, 0xfe]]))
            .await
            .err()
            .expect("a failure");
        assert_eq!(binary_failure.status(), Some(502));
        assert_eq!(
            binary_failure.to_string(),
            "Streaming request failed: the error body is not UTF-8"
        );
    });
}

#[rstest]
#[case::assistant_first_text(assistant_first_text)]
#[case::citation_index_relocation(citation_index_relocation)]
#[case::thread_id_metadata(thread_id_metadata)]
#[case::token_count_warning(token_count_warning)]
#[case::message_delta_stream(message_delta_stream)]
#[case::run_terminal_stream(run_terminal_stream)]
#[case::split_sse_event(split_sse_event)]
#[case::stream_thread_payload(stream_thread_payload)]
#[case::stream_status_failure(stream_status_failure)]
fn agent_results_cases(#[case] check: fn()) {
    check();
}
