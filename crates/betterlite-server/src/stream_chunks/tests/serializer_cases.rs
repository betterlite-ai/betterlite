use super::typed_chunk;

use crate::stream_chunks::ChunkKind;
use crate::stream_chunks::serialize::serialize_chunk;

fn assert_event_chunk(body: &str, expected: &str) {
    let chunk = typed_chunk(ChunkKind::Event, body);
    assert_eq!(serialize_chunk(&chunk), expected);
}

#[test]
fn serialize_text_keeps_dictionary_alternative_nulls() {
    assert_event_chunk(
        r#"{"type": "response.created", "response": {"id": "resp_text_dict", "created_at": 40, "output": [], "text": {"format": {"type": "future_format", "nullable": null}, "nullable": null}}}"#,
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_text_dict","created_at":40,"output":[],"#,
            r#""text":{"format":{"type":"future_format","nullable":null},"nullable":null}}}"#
        ),
    );
    assert_event_chunk(
        r#"{"type": "response.created", "response": {"id": "resp_text_schema", "created_at": 41, "output": [], "text": {"format": {"type": "json_schema", "name": "n", "nullable": null}, "nullable": null}}}"#,
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_text_schema","created_at":41,"output":[],"#,
            r#""text":{"format":{"type":"json_schema","name":"n","nullable":null},"nullable":null}}}"#
        ),
    );
    assert_event_chunk(
        r#"{"type": "response.created", "response": {"id": "resp_text_bad", "created_at": 42, "output": [], "text": {"format": {"type": "text"}, "verbosity": "loud", "nullable": null}}}"#,
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_text_bad","created_at":42,"output":[],"#,
            r#""text":{"format":{"type":"text"},"verbosity":"loud","nullable":null}}}"#
        ),
    );
    assert_event_chunk(
        r#"{"type": "response.created", "response": {"id": "resp_text_name", "created_at": 47, "output": [], "text": {"format": {"type": "json_schema", "name": 5, "schema": {"type": "object"}, "nullable": null}, "nullable": null}}}"#,
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_text_name","created_at":47,"output":[],"#,
            r#""text":{"format":{"type":"json_schema","name":5,"schema":{"type":"object"},"nullable":null},"nullable":null}}}"#
        ),
    );
}

#[test]
fn serialize_text_config_strips_declared_nulls() {
    assert_event_chunk(
        r#"{"type": "response.created", "response": {"id": "resp_text_ok", "created_at": 43, "output": [], "text": {"format": {"type": "json_schema", "name": "n", "schema": {"type": "object", "x": null}, "nullable": null, "extra": 5}, "verbosity": null, "nullable": null, "obj": {"a": null}}}}"#,
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_text_ok","created_at":43,"output":[],"#,
            r#""text":{"format":{"type":"json_schema","name":"n","schema":{"type":"object","x":null},"extra":5},"obj":{"a":null}}}}"#
        ),
    );
    assert_event_chunk(
        r#"{"type": "response.created", "response": {"id": "resp_text_text", "created_at": 44, "output": [], "text": {"format": {"type": "text", "extra": null}, "verbosity": null, "extra_null": null}}}"#,
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_text_text","created_at":44,"output":[],"#,
            r#""text":{"format":{"type":"text"}}}}"#
        ),
    );
    assert_event_chunk(
        r#"{"type": "response.created", "response": {"id": "resp_text_none", "created_at": 45, "output": [], "text": {"verbosity": "low", "nullable": null}}}"#,
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_text_none","created_at":45,"output":[],"#,
            r#""text":{"verbosity":"low"}}}"#
        ),
    );
    assert_event_chunk(
        r#"{"type": "response.created", "response": {"id": "resp_text_null", "created_at": 46, "output": [], "text": {"format": null, "nullable": null}}}"#,
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_text_null","created_at":46,"output":[],"#,
            r#""text":{}}}"#
        ),
    );
}

#[test]
fn serialize_audio_model_drops_null_extras() {
    let chunk = typed_chunk(
        ChunkKind::ModelStream,
        r#"{"id": "chatcmpl-audio", "object": "chat.completion.chunk", "created": 44, "model": "gpt-4o-audio-preview", "choices": [{"index": 0, "delta": {"role": "assistant", "content": null, "audio": {"id": "audio_1", "data": "ZGF0YQ==", "expires_at": 1700000000, "transcript": "hi", "extra_null": null, "extra_obj": {"a": null}}, "extra_null": null}, "finish_reason": null}]}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"id":"chatcmpl-audio","object":"chat.completion.chunk","created":44,"model":"gpt-4o-audio-preview","#,
            r#""choices":[{"index":0,"delta":{"role":"assistant","audio":{"id":"audio_1","data":"ZGF0YQ==","expires_at":1700000000,"transcript":"hi","extra_obj":{"a":null}}}}]}"#
        )
    );
}

#[test]
fn serialize_keeps_fields_the_fast_path_drops() {
    let chunk = typed_chunk(
        ChunkKind::ModelStream,
        r#"{"id": "chatcmpl-fast", "object": "chat.completion.chunk", "created": 45, "model": "gpt-4o", "citations": [{"url": "https://example.com", "extra_null": null}], "choices": [{"index": 0, "delta": {"role": "assistant", "content": "hi", "citations": [{"url": "https://example.com"}]}, "finish_reason": null}]}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"id":"chatcmpl-fast","object":"chat.completion.chunk","created":45,"model":"gpt-4o","#,
            r#""citations":[{"url":"https://example.com","extra_null":null}],"#,
            r#""choices":[{"index":0,"delta":{"role":"assistant","content":"hi","citations":[{"url":"https://example.com"}]}}]}"#
        )
    );
}
