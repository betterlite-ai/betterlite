use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrails::model_armor::streams::assembly::Assembly;
use crate::guardrails::model_armor::streams::hook::{StreamHook, StreamSettings, buffered_stream};
use crate::guardrails::streaming_rewrite::shapes::RequestData;

use super::support::{EmitReply, FakeWire, ScanReply, Setup, bytes, chunk, frame_text, value};

fn settings(fail_on_error: bool, mask_response_content: bool) -> StreamSettings {
    StreamSettings {
        fail_on_error,
        mask_response_content,
    }
}

fn request_data(json: &str) -> RequestData {
    serde_json::from_str(json).expect("the fixture is a request data map")
}

fn metadata_of(data: &RequestData) -> GuardrailValue {
    data.get("metadata")
        .cloned()
        .expect("the metadata bucket exists")
}

const CHAT_JSON: &str =
    r#"{"id": "chatcmpl-1", "choices": [{"message": {"content": "The secret"}}]}"#;

fn chat_wire(scan: ScanReply) -> FakeWire {
    FakeWire::new(Setup {
        chat: Some(Assembly::Model(value(CHAT_JSON))),
        model_text: Some("The secret".to_owned()),
        scan,
        logging: value(r#"{"logged": true}"#),
        ..Setup::default()
    })
}

#[tokio::test]
async fn assemble_failure_blocks_a_chat_stream_with_a_500_frame() {
    let wire = FakeWire::new(Setup::default());
    let mut data = request_data("{}");
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![chunk(r#"{"id": "chatcmpl-1", "choices": []}"#)],
    };
    let items = buffered_stream(hook).await.expect("the failure is framed");
    assert_eq!(
        frame_text(&items),
        "data: {\"code\":\"500\",\"message\":\"model-armor: streamed response could not be assembled for scanning, blocking it\"}\n\n"
    );
    assert_eq!(wire.applied(), vec!["model-armor".to_owned()]);
    assert!(wire.scans().is_empty());
    assert!(data.get("metadata").is_none());
}

#[tokio::test]
async fn assemble_failure_forwards_unscanned_when_fail_on_error_is_false() {
    let wire = FakeWire::new(Setup::default());
    let mut data = request_data("{}");
    let original = chunk(r#"{"id": "chatcmpl-1", "choices": []}"#);
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(false, true),
        chunks: vec![original.clone()],
    };
    let items = buffered_stream(hook)
        .await
        .expect("the stream is forwarded");
    assert_eq!(items, vec![original]);
    assert!(wire.applied().is_empty());
    assert!(data.get("metadata").is_none());
}

#[tokio::test]
async fn assembly_failure_blocks_an_anthropic_stream_with_anthropic_frames() {
    let frames = vec![bytes("event: error\ndata: {}\n\n")];
    let wire = FakeWire::new(Setup {
        anthropic: true,
        frames: frames.clone(),
        ..Setup::default()
    });
    let mut data = request_data("{}");
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![bytes("event: content_block_delta\ndata: {}\n\n")],
    };
    let items = buffered_stream(hook).await.expect("the failure is framed");
    assert_eq!(items, frames);
    assert_eq!(
        wire.frame_messages(),
        vec![
            "model-armor: streamed response could not be assembled for scanning, blocking it"
                .to_owned()
        ]
    );
}

#[tokio::test]
async fn assembly_failure_blocks_a_responses_stream_with_error_items() {
    let items = vec![chunk(r#"{"type": "error"}"#)];
    let wire = FakeWire::new(Setup {
        responses: Some(items.clone()),
        ..Setup::default()
    });
    let mut data = request_data("{}");
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![chunk(
            r#"{"type": "response.created", "response": {"id": "resp_1", "output": []}}"#,
        )],
    };
    let framed = buffered_stream(hook).await.expect("the failure is framed");
    assert_eq!(framed, items);
}

#[tokio::test]
async fn terminal_error_stream_is_yielded_without_scanning() {
    let wire = FakeWire::new(Setup {
        sse_error: true,
        chat: Some(Assembly::Model(value(CHAT_JSON))),
        model_text: Some("The secret".to_owned()),
        scan: ScanReply::Armor(value("{}")),
        ..Setup::default()
    });
    let mut data = request_data("{}");
    let original = bytes("data: {\"error\": {\"message\": \"blocked\"}}\n\n");
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![original.clone()],
    };
    let items = buffered_stream(hook)
        .await
        .expect("the refusal passes through");
    assert_eq!(items, vec![original]);
    assert!(wire.scans().is_empty());
    assert!(wire.applied().is_empty());
}

#[tokio::test]
async fn empty_stream_is_yielded_without_scanning() {
    let wire = chat_wire(ScanReply::Armor(value("{}")));
    let mut data = request_data("{}");
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: Vec::new(),
    };
    let items = buffered_stream(hook)
        .await
        .expect("the empty stream passes through");
    assert!(items.is_empty());
    assert!(wire.scans().is_empty());
}

#[tokio::test]
async fn scanned_content_reaches_the_wire() {
    let wire = chat_wire(ScanReply::Armor(value("{}")));
    let mut data = request_data("{}");
    let original = chunk(r#"{"id": "chatcmpl-1", "choices": []}"#);
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![original.clone()],
    };
    let items = buffered_stream(hook)
        .await
        .expect("the stream passes through");
    assert_eq!(items, vec![original]);
    assert_eq!(wire.scans(), vec!["The secret".to_owned()]);
    assert_eq!(wire.applied(), vec!["model-armor".to_owned()]);
    assert_eq!(
        metadata_of(&data),
        value(r#"{"_model_armor_response": {"logged": true}, "_model_armor_status": "success"}"#)
    );
}

#[tokio::test]
async fn empty_content_skips_the_scan() {
    let wire = FakeWire::new(Setup {
        chat: Some(Assembly::Model(value(r#"{"choices": []}"#))),
        model_text: Some(String::new()),
        ..Setup::default()
    });
    let mut data = request_data("{}");
    let original = chunk(r#"{"id": "chatcmpl-1", "choices": []}"#);
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![original.clone()],
    };
    let items = buffered_stream(hook)
        .await
        .expect("the stream passes through");
    assert_eq!(items, vec![original]);
    assert!(wire.scans().is_empty());
    assert!(data.get("metadata").is_none());
}

#[tokio::test]
async fn blocked_stream_frames_the_error_and_records_metadata() {
    let wire = FakeWire::new(Setup {
        chat: Some(Assembly::Model(value(CHAT_JSON))),
        model_text: Some("The secret".to_owned()),
        scan: ScanReply::Armor(value(r#"{"sanitizationResult": {}}"#)),
        block: true,
        logging: value(r#"{"logged": true}"#),
        block_detail: value(r#"{"error": "Streaming response blocked by Model Armor"}"#),
        ..Setup::default()
    });
    let mut data = request_data("{}");
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![chunk(r#"{"id": "chatcmpl-1", "choices": []}"#)],
    };
    let items = buffered_stream(hook).await.expect("the block is framed");
    assert_eq!(
        frame_text(&items),
        "data: {\"code\":\"400\",\"message\":\"Streaming response blocked by Model Armor\"}\n\n"
    );
    assert_eq!(wire.applied(), vec!["model-armor".to_owned()]);
    assert_eq!(
        metadata_of(&data),
        value(r#"{"_model_armor_response": {"logged": true}, "_model_armor_status": "blocked"}"#)
    );
}

#[tokio::test]
async fn sanitized_stream_rewrites_the_body_and_emits_the_chunks() {
    let emitted = vec![chunk(
        r#"{"id": "chatcmpl-1", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "[REDACTED]"}}]}"#,
    )];
    let wire = FakeWire::new(Setup {
        chat: Some(Assembly::Model(value(
            r#"{"id": "chatcmpl-1", "choices": [{"message": {"content": "The secret"}}, {"message": {"content": ""}}]}"#,
        ))),
        model_text: Some("The secret".to_owned()),
        scan: ScanReply::Armor(value(r#"{"sanitizationResult": {}}"#)),
        sanitized: Some("[REDACTED]".to_owned()),
        logging: value(r#"{"logged": true}"#),
        emit: EmitReply::Chunks(emitted.clone()),
        ..Setup::default()
    });
    let mut data = request_data("{}");
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![chunk(r#"{"id": "chatcmpl-1", "choices": []}"#)],
    };
    let items = buffered_stream(hook).await.expect("the rewrite is emitted");
    assert_eq!(items, emitted);
    assert_eq!(
        wire.emitted(),
        vec![value(
            r#"{"id": "chatcmpl-1", "choices": [{"message": {"content": "[REDACTED]"}}, {"message": {"content": ""}}]}"#
        )]
    );
    assert_eq!(
        metadata_of(&data),
        value(r#"{"_model_armor_response": {"logged": true}, "_model_armor_status": "success"}"#)
    );
}

#[tokio::test]
async fn rewrite_emit_failure_is_raised_when_fail_on_error_is_true() {
    let wire = FakeWire::new(Setup {
        chat: Some(Assembly::Model(value(CHAT_JSON))),
        model_text: Some("The secret".to_owned()),
        scan: ScanReply::Armor(value("{}")),
        sanitized: Some("[REDACTED]".to_owned()),
        emit: EmitReply::Fail,
        ..Setup::default()
    });
    let mut data = request_data("{}");
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![chunk(r#"{"id": "chatcmpl-1", "choices": []}"#)],
    };
    assert!(buffered_stream(hook).await.is_err());
}

#[tokio::test]
async fn rewrite_emit_failure_forwards_when_fail_on_error_is_false() {
    let wire = FakeWire::new(Setup {
        chat: Some(Assembly::Model(value(CHAT_JSON))),
        model_text: Some("The secret".to_owned()),
        scan: ScanReply::Armor(value("{}")),
        sanitized: Some("[REDACTED]".to_owned()),
        emit: EmitReply::Fail,
        ..Setup::default()
    });
    let mut data = request_data("{}");
    let original = chunk(r#"{"id": "chatcmpl-1", "choices": []}"#);
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(false, true),
        chunks: vec![original.clone()],
    };
    let items = buffered_stream(hook)
        .await
        .expect("the stream is forwarded");
    assert_eq!(items, vec![original]);
}

#[tokio::test]
async fn scan_api_failure_blocks_with_a_500_frame() {
    let wire = chat_wire(ScanReply::Api(
        "Model Armor API error (upstream 500)".to_owned(),
    ));
    let mut data = request_data("{}");
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![chunk(r#"{"id": "chatcmpl-1", "choices": []}"#)],
    };
    let items = buffered_stream(hook).await.expect("the failure is framed");
    assert_eq!(
        frame_text(&items),
        "data: {\"code\":\"500\",\"message\":\"Model Armor API error (upstream 500)\"}\n\n"
    );
    assert!(data.get("metadata").is_none());
    assert!(wire.applied().is_empty());
}

#[tokio::test]
async fn scan_api_failure_forwards_when_fail_on_error_is_false() {
    let wire = chat_wire(ScanReply::Api(
        "Model Armor API error (upstream 500)".to_owned(),
    ));
    let mut data = request_data("{}");
    let original = chunk(r#"{"id": "chatcmpl-1", "choices": []}"#);
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(false, true),
        chunks: vec![original.clone()],
    };
    let items = buffered_stream(hook)
        .await
        .expect("the stream is forwarded");
    assert_eq!(items, vec![original]);
    assert!(data.get("metadata").is_none());
}

#[tokio::test]
async fn unexpected_scan_failure_is_raised_when_fail_on_error_is_true() {
    let wire = chat_wire(ScanReply::Unexpected);
    let mut data = request_data("{}");
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![chunk(r#"{"id": "chatcmpl-1", "choices": []}"#)],
    };
    assert!(buffered_stream(hook).await.is_err());
}

#[tokio::test]
async fn unexpected_scan_failure_forwards_when_fail_on_error_is_false() {
    let wire = chat_wire(ScanReply::Unexpected);
    let mut data = request_data("{}");
    let original = chunk(r#"{"id": "chatcmpl-1", "choices": []}"#);
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(false, true),
        chunks: vec![original.clone()],
    };
    let items = buffered_stream(hook)
        .await
        .expect("the stream is forwarded");
    assert_eq!(items, vec![original]);
}

#[tokio::test]
async fn model_text_failure_is_raised_regardless_of_fail_on_error() {
    let wire = FakeWire::new(Setup {
        chat: Some(Assembly::Model(value(CHAT_JSON))),
        ..Setup::default()
    });
    let mut data = request_data("{}");
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(false, true),
        chunks: vec![chunk(r#"{"id": "chatcmpl-1", "choices": []}"#)],
    };
    assert!(buffered_stream(hook).await.is_err());
}

#[tokio::test]
async fn metadata_bucket_is_litellm_metadata_when_present() {
    let wire = chat_wire(ScanReply::Armor(value("{}")));
    let mut data =
        request_data(r#"{"litellm_metadata": {"existing": 1}, "metadata": {"untouched": 1}}"#);
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![chunk(r#"{"id": "chatcmpl-1", "choices": []}"#)],
    };
    buffered_stream(hook)
        .await
        .expect("the stream passes through");
    assert_eq!(
        data.get("litellm_metadata").cloned(),
        Some(value(
            r#"{"existing": 1, "_model_armor_response": {"logged": true}, "_model_armor_status": "success"}"#
        ))
    );
    assert_eq!(
        data.get("metadata").cloned(),
        Some(value(r#"{"untouched": 1}"#))
    );
}

#[tokio::test]
async fn metadata_bucket_replaces_a_non_object_value() {
    let wire = chat_wire(ScanReply::Armor(value("{}")));
    let mut data = request_data(r#"{"metadata": "text"}"#);
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(true, true),
        chunks: vec![chunk(r#"{"id": "chatcmpl-1", "choices": []}"#)],
    };
    buffered_stream(hook)
        .await
        .expect("the stream passes through");
    assert_eq!(
        metadata_of(&data),
        value(r#"{"_model_armor_response": {"logged": true}, "_model_armor_status": "success"}"#)
    );
}

#[tokio::test]
async fn assembly_failure_writes_no_metadata_when_forwarding() {
    let wire = FakeWire::new(Setup::default());
    let mut data = request_data("{}");
    let hook = StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: "model-armor",
        settings: settings(false, true),
        chunks: vec![chunk(r#"{"id": "chatcmpl-1", "choices": []}"#)],
    };
    buffered_stream(hook)
        .await
        .expect("the stream is forwarded");
    assert!(data.is_empty());
}
