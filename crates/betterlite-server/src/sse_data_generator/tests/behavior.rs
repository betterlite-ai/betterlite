use super::*;

#[test]
fn raw_frames_flush() {
    let data = [("_litellm_raw_sse_stream".to_owned(), Json::Bool(true))]
        .into_iter()
        .collect();
    let frames = collect_frames(
        vec![
            Ok(StreamItem::Bytes(bytes::Bytes::from_static(
                b"data: one\n\n",
            ))),
            Ok(StreamItem::Text("data: two".to_owned())),
        ],
        data,
    );
    assert_eq!(frames, ["data: one\n\n", "data: two\n\n", DONE_FRAME]);
}

#[test]
fn nonraw_relay_skip_done() {
    let data = [(
        "_litellm_skip_openai_stream_done".to_owned(),
        Json::Bool(true),
    )]
    .into_iter()
    .collect();
    let frames = collect_frames(
        vec![
            Ok(StreamItem::Bytes(bytes::Bytes::from_static(
                b"event: update\n\n",
            ))),
            Ok(StreamItem::Bytes(bytes::Bytes::from_static(
                b"data: payload",
            ))),
            Ok(StreamItem::Bytes(bytes::Bytes::from_static(b": ping"))),
        ],
        data,
    );
    assert_eq!(
        frames,
        ["event: update\n\n", "data: payload\n\n", ": ping\n\n"]
    );
}

#[test]
fn text_error_before_done() {
    let frames = collect_frames(
        vec![Ok(StreamItem::Text("data: failed".to_owned()))],
        JsonObject::new(),
    );
    assert_eq!(frames, ["data: failed", DONE_FRAME]);
}

#[test]
fn plain_failure_frame() {
    let failure = StreamFailure {
        message: "upstream failed".to_owned(),
        error_type: FailureType::Missing,
        param: FailureParam::Missing,
        status_code: None,
        kind: FailureKind::Other,
    };
    let frames = collect_frames(vec![Err(failure)], JsonObject::new());
    assert_eq!(frames.len(), 1);
    assert!(frames[0].contains("upstream failed"));
    assert!(!frames[0].contains("[DONE]"));
}

#[test]
fn hooks_text_and_logging() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let generator = SseDataGenerator {
        hooks: Arc::new(RecordingHooks {
            events: events.clone(),
            wrap: true,
            per_chunk: true,
            chunk_fails: false,
            failure_hook_fails: false,
        }),
        error_state: Arc::new(RecordingErrorSource {
            events: events.clone(),
            failure_frame: None,
        }),
        cleanup: Arc::new(NoCleanup),
        ..test_generator(None)
    };
    let frames = collect_with_generator(
        generator,
        vec![Ok(model_chunk("one")), Ok(model_chunk("two"))],
        JsonObject::new(),
    );
    assert_eq!(
        events.lock().expect("events").as_slice(),
        [
            "needs-wrap",
            "wrap",
            "needs-chunk",
            "chunk:<none>",
            "needs-chunk",
            "chunk:one",
        ]
    );
    assert!(frames[0].starts_with("data: {"));
}

#[test]
fn deferred_logging_once() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let generator = SseDataGenerator {
        hooks: Arc::new(RecordingHooks {
            events: events.clone(),
            wrap: false,
            per_chunk: false,
            chunk_fails: false,
            failure_hook_fails: false,
        }),
        error_state: Arc::new(RecordingErrorSource {
            events: events.clone(),
            failure_frame: None,
        }),
        cleanup: Arc::new(NoCleanup),
        ..test_generator(None)
    };
    collect_with_generator(generator, Vec::new(), JsonObject::new());
    assert_eq!(
        events.lock().expect("events").as_slice(),
        ["needs-wrap", "deferred"]
    );
}

#[test]
fn lazy_setup() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let generator = SseDataGenerator {
        hooks: Arc::new(RecordingHooks {
            events: events.clone(),
            wrap: true,
            per_chunk: true,
            chunk_fails: false,
            failure_hook_fails: false,
        }),
        error_state: Arc::new(RecordingErrorSource {
            events: events.clone(),
            failure_frame: None,
        }),
        cleanup: Arc::new(NoCleanup),
        ..test_generator(None)
    };
    let body = generator.generate(GeneratorInput {
        response: StreamSource {
            items: stream::pending().boxed(),
            hidden: JsonObject::new(),
            snapshot: None,
        },
        key: (),
        request_data: JsonObject::new(),
        request: None,
        responses_stream_errors: true,
    });
    assert!(events.lock().expect("events").is_empty());
    drop(body);
    assert!(events.lock().expect("events").is_empty());
}

#[test]
fn chunk_hook_failure() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let generator = SseDataGenerator {
        hooks: Arc::new(RecordingHooks {
            events: events.clone(),
            wrap: false,
            per_chunk: true,
            chunk_fails: true,
            failure_hook_fails: false,
        }),
        error_state: Arc::new(NoErrorState),
        cleanup: Arc::new(NoCleanup),
        ..test_generator(None)
    };
    let frames = collect_with_generator(generator, vec![Ok(model_chunk("bad"))], JsonObject::new());
    assert!(frames[0].contains("guardrail failed"));
    assert_eq!(
        events.lock().expect("events").as_slice(),
        [
            "new-state",
            "needs-wrap",
            "needs-chunk",
            "chunk:<none>",
            "failure:guardrail failed",
        ]
    );
}

#[test]
fn fallback_after_partial() {
    let mut generator = test_generator(None);
    generator.expose_fallback_errors = true;
    let source = StreamSource {
        items: stream::iter([Ok(StreamItem::Text("data: partial".to_owned()))]).boxed(),
        hidden: fallback_hidden(),
        snapshot: None,
    };
    let request_data = [
        ("_litellm_raw_sse_stream".to_owned(), Json::Bool(true)),
        ("include_fallback_errors".to_owned(), Json::Bool(true)),
    ]
    .into_iter()
    .collect();
    let frames = run_generator(generator, source, request_data, false);
    assert!(frames[0].contains("litellm-fallback-metadata"));
    let event_json = frames[0]
        .strip_prefix("data: ")
        .expect("fallback data frame")
        .trim();
    let payload: serde_json::Value = serde_json::from_str(event_json).expect("fallback JSON");
    assert!(payload["created"].as_i64().expect("created timestamp") > 1);
    assert_eq!(frames[1], "data: partial\n\n");
    assert_eq!(frames[2], DONE_FRAME);
}

#[test]
fn fallback_latched_once() {
    let mut generator = test_generator(None);
    generator.expose_fallback_errors = true;
    let mut first = model_chunk("first");
    let mut later = model_chunk("later");
    if let StreamItem::Chunk(chunk) = &mut first {
        chunk.hidden = fallback_hidden();
    }
    if let StreamItem::Chunk(chunk) = &mut later {
        chunk.hidden = fallback_hidden();
        if let Some(Json::Object(headers)) = chunk.hidden.get_mut("additional_headers") {
            headers.insert(
                "x-litellm-model-group".to_owned(),
                Json::Text("should-not-relatch".to_owned()),
            );
        }
    }
    let source = StreamSource {
        items: stream::iter([Ok(first), Ok(later)]).boxed(),
        hidden: JsonObject::new(),
        snapshot: None,
    };
    let request_data = [("include_fallback_errors".to_owned(), Json::Bool(true))]
        .into_iter()
        .collect();
    let frames = run_generator(generator, source, request_data, false);
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame.contains("litellm-fallback-metadata"))
            .count(),
        1
    );
    assert!(frames[1].contains("fallback-model"));
}

#[test]
fn strip_keeps_fallback() {
    let mut generator = test_generator(None);
    generator.expose_fallback_errors = true;
    let usage = StreamItem::Chunk(StreamChunk {
        body: [("choices".to_owned(), Json::List(Vec::new()))]
            .into_iter()
            .collect(),
        kind: ChunkKind::ModelStream,
        hidden: JsonObject::new(),
    });
    let source = StreamSource {
        items: stream::iter([Ok(usage)]).boxed(),
        hidden: fallback_hidden(),
        snapshot: None,
    };
    let request_data = [
        ("_litellm_strip_stream_usage".to_owned(), Json::Bool(true)),
        ("include_fallback_errors".to_owned(), Json::Bool(true)),
    ]
    .into_iter()
    .collect();
    let frames = run_generator(generator, source, request_data, false);
    assert_eq!(frames.len(), 2);
    assert!(frames[0].contains("litellm-fallback-metadata"));
    assert_eq!(frames[1], DONE_FRAME);
}

#[test]
fn responses_state_done() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let hooks = Arc::new(RecordingHooks {
        events: events.clone(),
        wrap: false,
        per_chunk: false,
        chunk_fails: false,
        failure_hook_fails: false,
    });
    let generator = SseDataGenerator {
        hooks,
        error_state: Arc::new(RecordingErrorSource {
            events: events.clone(),
            failure_frame: None,
        }),
        cleanup: Arc::new(NoCleanup),
        ..test_generator(None)
    };
    let source = StreamSource {
        items: stream::iter([Ok(model_chunk("x")), Err(other_failure("end"))]).boxed(),
        hidden: JsonObject::new(),
        snapshot: None,
    };
    let frames = run_generator(generator, source, JsonObject::new(), true);
    assert!(frames[0].starts_with("data: {"));
    assert_eq!(frames[1], DONE_FRAME);
    assert_eq!(
        events.lock().expect("events").as_slice(),
        [
            "new-state",
            "needs-wrap",
            "needs-chunk",
            "observe",
            "mark",
            "failure:end",
            "format",
        ]
    );
}

#[test]
fn responses_failure_frame() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let generator = SseDataGenerator {
        hooks: Arc::new(NoHooks),
        error_state: Arc::new(RecordingErrorSource {
            events,
            failure_frame: Some("event: response.failed\n\n".to_owned()),
        }),
        cleanup: Arc::new(NoCleanup),
        ..test_generator(None)
    };
    let frames = run_generator(
        generator,
        StreamSource {
            items: stream::iter([Err(other_failure("terminal"))]).boxed(),
            hidden: JsonObject::new(),
            snapshot: None,
        },
        JsonObject::new(),
        true,
    );
    assert_eq!(frames, ["event: response.failed\n\n", DONE_FRAME]);
}

#[test]
fn cleanup_normal_and_drop() {
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let generator = SseDataGenerator {
        hooks: Arc::new(NoHooks),
        error_state: Arc::new(NoErrorState),
        cleanup: Arc::new(RecordingCleanup(outcomes.clone())),
        ..test_generator(None)
    };
    let source = StreamSource {
        items: stream::iter([Ok(model_chunk("normal"))]).boxed(),
        hidden: JsonObject::new(),
        snapshot: Some(Arc::new(EmptySnapshot)),
    };
    let body = generator.generate(GeneratorInput {
        response: source,
        key: (),
        request_data: JsonObject::new(),
        request: None,
        responses_stream_errors: false,
    });
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(body.collect::<Vec<_>>());
    assert_eq!(
        outcomes.lock().expect("cleanup").as_slice(),
        [(true, false, true)]
    );

    let source = StreamSource {
        items: stream::iter([Ok(model_chunk("first")), Ok(model_chunk("second"))]).boxed(),
        hidden: JsonObject::new(),
        snapshot: None,
    };
    let body = generator.generate(GeneratorInput {
        response: source,
        key: (),
        request_data: JsonObject::new(),
        request: None,
        responses_stream_errors: false,
    });
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            let mut body = body;
            body.next().await.expect("first frame").expect("frame");
            drop(body);
        });
    assert_eq!(
        outcomes.lock().expect("cleanup").as_slice(),
        [(true, false, true), (false, true, false)]
    );
}

#[test]
fn http_failure_cleanup() {
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let generator = SseDataGenerator {
        hooks: Arc::new(NoHooks),
        error_state: Arc::new(NoErrorState),
        cleanup: Arc::new(RecordingCleanup(outcomes.clone())),
        ..test_generator(None)
    };
    let mut failure = other_failure("http failed");
    failure.kind = FailureKind::Http;
    let source = StreamSource {
        items: stream::iter([Err(failure)]).boxed(),
        hidden: JsonObject::new(),
        snapshot: None,
    };
    let body = generator.generate(GeneratorInput {
        response: source,
        key: (),
        request_data: JsonObject::new(),
        request: None,
        responses_stream_errors: false,
    });
    let frames = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(body.collect::<Vec<_>>());
    assert!(frames[0].is_err());
    assert_eq!(
        outcomes.lock().expect("cleanup").as_slice(),
        [(false, false, false)]
    );
}

#[test]
fn hook_error_cleanup() {
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let generator = SseDataGenerator {
        hooks: Arc::new(RecordingHooks {
            events,
            wrap: false,
            per_chunk: false,
            chunk_fails: false,
            failure_hook_fails: true,
        }),
        error_state: Arc::new(NoErrorState),
        cleanup: Arc::new(RecordingCleanup(outcomes.clone())),
        ..test_generator(None)
    };
    let source = StreamSource {
        items: stream::iter([Err(other_failure("upstream failed"))]).boxed(),
        hidden: JsonObject::new(),
        snapshot: None,
    };
    let body = generator.generate(GeneratorInput {
        response: source,
        key: (),
        request_data: JsonObject::new(),
        request: None,
        responses_stream_errors: false,
    });
    let output = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(body.collect::<Vec<_>>());
    assert!(output[0].is_err());
    assert_eq!(
        outcomes.lock().expect("cleanup").as_slice(),
        [(false, false, false)]
    );
}

#[test]
fn raw_overflow_error() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let generator = SseDataGenerator {
        hooks: Arc::new(RecordingHooks {
            events: events.clone(),
            wrap: false,
            per_chunk: false,
            chunk_fails: false,
            failure_hook_fails: false,
        }),
        error_state: Arc::new(NoErrorState),
        cleanup: Arc::new(NoCleanup),
        ..test_generator(None)
    };
    let mut input = b"data: complete\n\n".to_vec();
    input.extend(std::iter::repeat_n(b'x', MAX_RAW_SSE_BUFFER_CHARS + 1));
    let source = StreamSource {
        items: stream::iter([Ok(StreamItem::Bytes(Bytes::from(input)))]).boxed(),
        hidden: JsonObject::new(),
        snapshot: None,
    };
    let request_data = [("_litellm_raw_sse_stream".to_owned(), Json::Bool(true))]
        .into_iter()
        .collect();
    let body = generator.generate(GeneratorInput {
        response: source,
        key: (),
        request_data,
        request: None,
        responses_stream_errors: false,
    });
    let frames = collect_body(body);
    assert_eq!(frames[0], "data: complete\n\n");
    assert!(frames[1].contains("Raw SSE stream exceeded maximum buffered size"));
    assert_eq!(
        events.lock().expect("events").as_slice(),
        [
            "needs-wrap",
            "needs-chunk",
            "failure:Raw SSE stream exceeded maximum buffered size without a frame delimiter",
        ]
    );
}

#[test]
fn raw_limit_flush() {
    let input = Bytes::from(vec![b'x'; MAX_RAW_SSE_BUFFER_CHARS]);
    let frames = collect_frames(
        vec![Ok(StreamItem::Bytes(input))],
        [("_litellm_raw_sse_stream".to_owned(), Json::Bool(true))]
            .into_iter()
            .collect(),
    );
    assert_eq!(frames[0].len(), MAX_RAW_SSE_BUFFER_CHARS + 2);
    assert!(frames[0].ends_with("\n\n"));
    assert_eq!(frames[1], DONE_FRAME);
}

#[tokio::test(start_paused = true)]
async fn keepalive_ping() {
    let generator = test_generator(Some(crate::keepalive_interval::PingSetting::Seconds(1.0)));
    let source = StreamSource {
        items: stream::pending().boxed(),
        hidden: JsonObject::new(),
        snapshot: None,
    };
    let mut body = generator.generate(GeneratorInput {
        response: source,
        key: (),
        request_data: JsonObject::new(),
        request: None,
        responses_stream_errors: false,
    });
    let ping = body.next().await.expect("ping").expect("body frame");
    assert_eq!(ping, bytes::Bytes::from_static(b": ping\n\n"));
}

#[test]
fn emits_data_chunks_then_done() {
    let chunk = StreamChunk {
        body: [
            ("id".to_owned(), Json::Text("1".to_owned())),
            ("model".to_owned(), Json::Text("wrong".to_owned())),
            ("choices".to_owned(), Json::List(Vec::new())),
        ]
        .into_iter()
        .collect(),
        kind: ChunkKind::ModelStream,
        hidden: JsonObject::new(),
    };
    let mut expected = chunk.clone();
    expected
        .body
        .insert("model".to_owned(), Json::Text("requested".to_owned()));
    let frames = collect_frames(
        vec![Ok(StreamItem::Chunk(chunk))],
        [("model".to_owned(), Json::Text("requested".to_owned()))]
            .into_iter()
            .collect(),
    );
    assert_eq!(frames[0], sse_data_frame(&serialize_chunk(&expected)));
    assert_eq!(frames.last().map(String::as_str), Some(DONE_FRAME));
    assert_eq!(collect_frames(Vec::new(), JsonObject::new()), [DONE_FRAME]);
}
