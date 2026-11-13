use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::wire::StreamReply;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use bytes::Bytes;
use futures_util::StreamExt as _;

use crate::converse_events::ChunkProjection;
use crate::converse_events::response::ConverseChunk;
use crate::invoke_events::error::InvokeStreamError;
use crate::invoke_events::fake::{FakeResponse, FakeStream};
use crate::invoke_events::framing::decode_bedrock_event_json;
use crate::invoke_events::{
    InvokeChunk, InvokeStreamDecoder, bedrock_event_stream_decoder, stream_decoder,
};

use super::support::{
    MODEL, NOW, block_on, chunk_frame, clock, json, options, redactor, request_headers,
};

fn parse(decoder: &mut InvokeStreamDecoder, source: &str) -> InvokeChunk {
    decoder.chunk_parser(&json(source)).unwrap()
}

fn base(chunk: InvokeChunk) -> ChunkProjection {
    match chunk {
        InvokeChunk::Base(projection) => *projection,
        _ => panic!("expected the base decoder projection"),
    }
}

fn generic(chunk: InvokeChunk) -> GenericChunk {
    match base(chunk) {
        ChunkProjection::Generic(generic) => generic,
        ChunkProjection::Converse(_) => panic!("expected a generic chunk"),
    }
}

fn converse(chunk: InvokeChunk) -> ConverseChunk {
    match base(chunk) {
        ChunkProjection::Converse(converse) => converse,
        ChunkProjection::Generic(_) => panic!("expected a converse chunk"),
    }
}

fn anthropic(chunk: InvokeChunk) -> StreamEventChunk {
    match chunk {
        InvokeChunk::Anthropic(chunk) => *chunk,
        _ => panic!("expected the Anthropic projection"),
    }
}

fn reply(chunk: InvokeChunk) -> StreamReply {
    match chunk {
        InvokeChunk::DeepSeek(reply) | InvokeChunk::OpenAiCompatible(reply) => *reply,
        _ => panic!("expected a stream reply"),
    }
}

fn error(chunk: Result<InvokeChunk, InvokeStreamError>) -> InvokeStreamError {
    match chunk {
        Ok(_) => panic!("expected a projection error"),
        Err(error) => error,
    }
}

fn fake(source: &str, json_mode: Option<bool>) -> FakeStream {
    let response: FakeResponse = serde_json::from_str(source).unwrap();
    FakeStream {
        response,
        json_mode,
        is_done: false,
    }
}

fn tool_use_json(chunk: &GenericChunk) -> Json {
    serde_json::to_value(chunk.tool_use.as_ref()).unwrap()
}

fn byte_chunks(frames: &[Vec<u8>]) -> std::vec::IntoIter<Bytes> {
    frames
        .iter()
        .cloned()
        .map(Bytes::from)
        .collect::<Vec<_>>()
        .into_iter()
}

pub(super) fn claude3_invoke_stream() {
    let mut decoder = stream_decoder(Some("anthropic"), &options(Some(false), false));
    let start = anthropic(parse(
        &mut decoder,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"f","input":{}}}"#,
    ));
    let call = &start.choices[0].delta.tool_calls.as_ref().unwrap()[0];
    assert_eq!(call.id.as_deref(), Some("t1"));
    assert_eq!(call.function.name.as_deref(), Some("f"));
    assert_eq!(call.function.arguments, "");
    let text = anthropic(parse(
        &mut decoder,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
    ));
    assert_eq!(text.choices[0].delta.content, "hi");
    assert_eq!(text.id, format!("chatcmpl-{}", super::support::fixed_id()));
    let thinking = anthropic(parse(
        &mut decoder,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"plan"}}"#,
    ));
    assert_eq!(
        thinking.choices[0].delta.reasoning_content.as_deref(),
        Some("plan")
    );
    let signature = anthropic(parse(
        &mut decoder,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}"#,
    ));
    assert!(signature.choices[0].delta.thinking_blocks.is_some());
    let usage_start = anthropic(parse(
        &mut decoder,
        r#"{"type":"message_start","message":{"model":"claude","usage":{"input_tokens":1,"output_tokens":2}}}"#,
    ));
    assert!(usage_start.usage.is_some());
    assert_eq!(usage_start.model.as_deref(), Some("claude"));
    let finished = anthropic(parse(
        &mut decoder,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#,
    ));
    assert_eq!(finished.choices[0].finish_reason.as_deref(), Some("stop"));
    // the deviation in docs/deviations/B609.yaml: the streamed thinking text is not tokenized
    let details = finished.usage.unwrap().completion_tokens_details;
    assert_eq!(details.reasoning, Some(0));

    let mut converted = stream_decoder(Some("anthropic"), &options(Some(true), true));
    let suppressed = anthropic(parse(
        &mut converted,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"j","name":"json_tool_call","input":{}}}"#,
    ));
    assert!(suppressed.choices[0].delta.tool_calls.is_none());
    assert_eq!(suppressed.choices[0].delta.content, "");
}

pub(super) fn json_mode_tool_chunk() {
    let values = fake(
        r#"{"choices":[{"message":{"content":"raw","tool_calls":[{"id":"c1","type":"function","function":{"name":"json_tool_call","arguments":"{\"values\": {\"a\": 1}}"}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    let chunk = values.into_iter().next().unwrap().unwrap();
    assert_eq!(chunk.text, r#"{"a": 1}"#);
    assert!(chunk.tool_use.is_none());

    let whole = fake(
        r#"{"choices":[{"message":{"content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"json_tool_call","arguments":"{\"a\": 1}"}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    let chunk = whole.into_iter().next().unwrap().unwrap();
    assert_eq!(chunk.text, r#"{"a": 1}"#);

    let malformed = fake(
        r#"{"choices":[{"message":{"content":"kept","tool_calls":[{"id":"c1","type":"function","function":{"name":"json_tool_call","arguments":"not json"}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    let chunk = malformed.into_iter().next().unwrap().unwrap();
    assert_eq!(chunk.text, "not json");
    assert!(chunk.tool_use.is_none());

    let missing = fake(
        r#"{"choices":[{"message":{"content":"kept","tool_calls":[{"id":"c1","type":"function","function":{"name":"json_tool_call"}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    let chunk = missing.into_iter().next().unwrap().unwrap();
    assert_eq!(chunk.text, "kept");
    assert!(chunk.tool_use.is_none());

    let real = fake(
        r#"{"choices":[{"message":{"content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}},{"id":"c2","type":"function","function":{"name":"g","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(false),
    );
    let chunk = real.into_iter().next().unwrap().unwrap();
    assert_eq!(
        tool_use_json(&chunk),
        serde_json::json!({"id":"c1","type":"function","function":{"name":"f","arguments":"{}"},"index":0})
    );

    let empty_real = fake(
        r#"{"choices":[{"message":{"content":"x","tool_calls":[]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(false),
    );
    let chunk = empty_real.into_iter().next().unwrap().unwrap();
    assert_eq!(chunk.text, "x");
    assert!(chunk.tool_use.is_none());

    let empty_converted = fake(
        r#"{"choices":[{"message":{"content":"x","tool_calls":[]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    assert!(empty_converted.into_iter().next().unwrap().is_err());
}

pub(super) fn json_mode_stop() {
    let mut stream = fake(
        r#"{"choices":[{"message":{"content":"hi","tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}}]},"finish_reason":"max_tokens"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    let chunk = stream.next().unwrap().unwrap();
    assert!(chunk.is_finished);
    assert_eq!(chunk.finish_reason, "length");
    assert_eq!(chunk.index, Some(0));
    let usage = chunk.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 1);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 3);
    assert!(stream.next().is_none());

    let mut async_stream = fake(
        r#"{"choices":[{"message":{"content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        None,
    );
    let chunk = block_on(async_stream.next()).unwrap().unwrap();
    assert_eq!(chunk.finish_reason, "stop");
    assert!(block_on(async_stream.next()).is_none());

    let unknown = fake(
        r#"{"choices":[{"message":{"content":"hi"},"finish_reason":"weird"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        None,
    );
    assert_eq!(
        unknown.into_iter().next().unwrap().unwrap().finish_reason,
        "stop"
    );
    let absent = fake(
        r#"{"choices":[{"message":{"content":"hi"},"finish_reason":null}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        None,
    );
    assert_eq!(
        absent.into_iter().next().unwrap().unwrap().finish_reason,
        "stop"
    );
}

pub(super) fn empty_tool_input() {
    let stream = fake(
        r#"{"choices":[{"message":{"content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":""}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(false),
    );
    let chunk = stream.into_iter().next().unwrap().unwrap();
    assert_eq!(
        tool_use_json(&chunk),
        serde_json::json!({"id":"c1","type":"function","function":{"name":"f","arguments":""},"index":0})
    );

    let missing_usage = fake(
        r#"{"choices":[{"message":{"content":"x"},"finish_reason":"stop"}]}"#,
        None,
    );
    let error = missing_usage.into_iter().next().unwrap().unwrap_err();
    assert_eq!(error.status(), None);
    assert!(error.to_string().starts_with("Failed to decode chunk:"));

    let missing_choices = fake(
        r#"{"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        None,
    );
    let error = missing_choices.into_iter().next().unwrap().unwrap_err();
    assert!(error.to_string().starts_with("Failed to decode chunk:"));
}

pub(super) fn deepseek_stream_chunks() {
    let mut decoder = stream_decoder(Some("deepseek_r1"), &options(None, false));
    let thinking = reply(parse(
        &mut decoder,
        r#"{"generation":"thinking","stop_reason":"stop","prompt_token_count":1,"generation_token_count":2}"#,
    ));
    let delta = &thinking.choices[0].delta;
    assert_eq!(delta.reasoning_content.as_deref(), Some("thinking"));
    assert_eq!(delta.content, None);
    assert_eq!(thinking.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        thinking.usage,
        Some(serde_json::json!({"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}))
    );
    assert_eq!(
        thinking.id,
        format!("chatcmpl-{}", super::support::fixed_id())
    );
    assert_eq!(thinking.created, NOW);
    assert_eq!(thinking.object, "chat.completion.chunk");
    assert_eq!(thinking.model, None);

    let transition = reply(parse(
        &mut decoder,
        r#"{"generation":"</think>","stop_reason":"stop","prompt_token_count":0,"generation_token_count":0}"#,
    ));
    assert_eq!(transition.choices[0].delta.content.as_deref(), Some(""));
    assert_eq!(transition.choices[0].delta.reasoning_content, None);

    let answer = reply(parse(
        &mut decoder,
        r#"{"generation":"answer","stop_reason":"max_tokens","prompt_token_count":0,"generation_token_count":0}"#,
    ));
    assert_eq!(answer.choices[0].delta.content.as_deref(), Some("answer"));
    assert_eq!(answer.choices[0].delta.reasoning_content, None);
    assert_eq!(
        answer.choices[0].finish_reason.as_deref(),
        Some("max_tokens")
    );

    let literal = reply(parse(
        &mut decoder,
        r#"{"generation":"</think>","stop_reason":null,"prompt_token_count":0,"generation_token_count":0}"#,
    ));
    assert_eq!(
        literal.choices[0].delta.content.as_deref(),
        Some("</think>")
    );
    assert_eq!(literal.choices[0].finish_reason, None);

    let first = reply(parse(
        &mut decoder,
        r#"{"generation":"a</think>","stop_reason":"stop","prompt_token_count":0,"generation_token_count":0}"#,
    ));
    assert_eq!(
        first.choices[0].delta.reasoning_content.as_deref(),
        Some("a</think>")
    );

    let mut negative = stream_decoder(Some("deepseek_r1"), &options(None, false));
    let counted = reply(parse(
        &mut negative,
        r#"{"generation":"x","stop_reason":"stop","prompt_token_count":-1,"generation_token_count":2}"#,
    ));
    assert_eq!(
        counted.usage,
        Some(serde_json::json!({"prompt_tokens":-1,"completion_tokens":2,"total_tokens":1}))
    );

    assert!(
        negative
            .chunk_parser(&json(
                r#"{"stop_reason":"stop","prompt_token_count":0,"generation_token_count":0}"#
            ))
            .is_err()
    );

    let mut failures = stream_decoder(Some("deepseek_r1"), &options(None, false));
    assert!(
        failures
            .chunk_parser(&json(
                r#"{"generation":"x","prompt_token_count":0,"generation_token_count":0}"#
            ))
            .is_err()
    );
    assert!(
        failures
            .chunk_parser(&json(r#"{"generation":"x","stop_reason":"stop"}"#))
            .is_ok()
    );
}

pub(super) fn stream_selector_variants() {
    let probe = json(
        r#"{"generation":"x","stop_reason":"stop","prompt_token_count":0,"generation_token_count":0}"#,
    );
    let mut explicit = stream_decoder(None, &options(Some(true), false));
    assert_eq!(generic(explicit.chunk_parser(&probe).unwrap()).text, "x");
    let mut legacy = bedrock_event_stream_decoder(None, &options(Some(true), false));
    assert_eq!(generic(legacy.chunk_parser(&probe).unwrap()).text, "x");

    let converse_probe = json(
        r#"{"start":{"toolUse":{"toolUseId":"j","name":"json_tool_call"}},"contentBlockIndex":0}"#,
    );
    let mut explicit_json = stream_decoder(None, &options(Some(true), false));
    let suppressed = converse(explicit_json.chunk_parser(&converse_probe).unwrap());
    assert!(suppressed.delta.tool_calls.is_none());
    assert_eq!(
        suppressed.id,
        format!("chatcmpl-{}", super::support::fixed_id())
    );
    let mut legacy_json = bedrock_event_stream_decoder(None, &options(Some(true), false));
    assert!(
        converse(legacy_json.chunk_parser(&converse_probe).unwrap())
            .delta
            .tool_calls
            .is_some()
    );

    let moonshot =
        json(r#"{"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#);
    let mut explicit_moonshot = stream_decoder(Some("moonshot"), &options(None, false));
    assert_eq!(
        reply(explicit_moonshot.chunk_parser(&moonshot).unwrap()).choices[0]
            .delta
            .content
            .as_deref(),
        Some("hi")
    );
    let mut legacy_moonshot = bedrock_event_stream_decoder(Some("moonshot"), &options(None, false));
    assert_eq!(
        generic(legacy_moonshot.chunk_parser(&moonshot).unwrap()).text,
        ""
    );

    let deepseek = json(
        r#"{"generation":"x","stop_reason":"stop","prompt_token_count":0,"generation_token_count":0}"#,
    );
    let mut explicit_deepseek = stream_decoder(Some("deepseek_r1"), &options(None, false));
    assert_eq!(
        reply(explicit_deepseek.chunk_parser(&deepseek).unwrap()).choices[0]
            .delta
            .reasoning_content
            .as_deref(),
        Some("x")
    );
    let mut legacy_deepseek =
        bedrock_event_stream_decoder(Some("deepseek_r1"), &options(None, false));
    assert_eq!(
        reply(legacy_deepseek.chunk_parser(&deepseek).unwrap()).choices[0]
            .delta
            .reasoning_content
            .as_deref(),
        Some("x")
    );

    let anthropic_event = json(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
    );
    let mut explicit_anthropic = stream_decoder(Some("anthropic"), &options(None, false));
    assert_eq!(
        anthropic(explicit_anthropic.chunk_parser(&anthropic_event).unwrap()).choices[0]
            .delta
            .content,
        "hi"
    );
    let mut legacy_anthropic =
        bedrock_event_stream_decoder(Some("anthropic"), &options(None, false));
    assert_eq!(
        anthropic(legacy_anthropic.chunk_parser(&anthropic_event).unwrap()).choices[0]
            .delta
            .content,
        "hi"
    );
}

pub(super) fn openai_compatible_invoke_stream() {
    let mut decoder = stream_decoder(Some("moonshot"), &options(None, false));
    let chunk = reply(parse(
        &mut decoder,
        r#"{"id":"chatcmpl-x","created":5,"model":"kimi","choices":[{"index":0,"delta":{"role":"assistant","content":"hi","tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
    ));
    assert_eq!(chunk.id, "chatcmpl-x");
    assert_eq!(chunk.created, 5);
    assert_eq!(chunk.model.as_deref(), Some("kimi"));
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
    assert_eq!(
        chunk.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
    assert_eq!(
        chunk.usage,
        Some(serde_json::json!({"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}))
    );

    let generated = reply(parse(
        &mut decoder,
        r#"{"choices":[{"index":0,"delta":{"content":"x"}}]}"#,
    ));
    assert_eq!(
        generated.id,
        format!("chatcmpl-{}", super::support::fixed_id())
    );
    assert_eq!(generated.created, NOW);

    let failure = error(decoder.chunk_parser(&json(r#"{"error":{"message":"boom","code":400}}"#)));
    assert_eq!(failure.status(), Some(400));

    let mut legacy = bedrock_event_stream_decoder(Some("moonshot"), &options(None, false));
    assert_eq!(
        generic(parse(
            &mut legacy,
            r#"{"choices":[{"index":0,"delta":{"content":"x"}}]}"#
        ))
        .text,
        ""
    );
}

pub(super) fn framing_iteration_and_eof() {
    let redactor = redactor();
    let headers = request_headers();

    let mut decoder = stream_decoder(None, &options(None, false));
    let frames = vec![
        chunk_frame(r#"{"outputText":"a"}"#),
        chunk_frame(r#"{"outputText":"b"}"#),
    ];
    let chunks = byte_chunks(&frames);
    let outcomes: Vec<_> = decoder
        .framed(chunks, Some(headers.clone()), &redactor)
        .collect();
    let texts: Vec<String> = outcomes
        .into_iter()
        .map(|outcome| generic(outcome.unwrap()).text)
        .collect();
    assert_eq!(texts, ["a", "b"]);

    let mut decoder = stream_decoder(None, &options(None, false));
    let mut split = frames[0].clone();
    let tail = split.split_off(split.len() - 3);
    let chunks = byte_chunks(&[split, tail]);
    let outcomes: Vec<_> = decoder
        .framed(chunks, Some(headers.clone()), &redactor)
        .collect();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        generic(outcomes.into_iter().next().unwrap().unwrap()).text,
        "a"
    );

    let mut decoder = stream_decoder(None, &options(None, false));
    let empty = vec![chunk_frame(""), chunk_frame("{}")];
    let outcomes: Vec<_> = decoder
        .framed(byte_chunks(&empty), Some(headers.clone()), &redactor)
        .collect();
    assert_eq!(outcomes.len(), 1);
    let only = generic(outcomes.into_iter().next().unwrap().unwrap());
    assert_eq!(only.text, "");
    assert!(!only.is_finished);

    let mut decoder = stream_decoder(None, &options(None, false));
    let late_error = vec![
        chunk_frame(r#"{"outputText":"first"}"#),
        chunk_frame("{not json"),
        chunk_frame(r#"{"outputText":"never"}"#),
    ];
    let outcomes: Vec<_> = decoder
        .framed(byte_chunks(&late_error), Some(headers.clone()), &redactor)
        .collect();
    assert_eq!(outcomes.len(), 2);
    let mut outcomes = outcomes.into_iter();
    assert_eq!(generic(outcomes.next().unwrap().unwrap()).text, "first");
    let failure = error(outcomes.next().unwrap());
    assert_eq!(failure.status(), None);
    assert!(failure.to_string().contains("not valid JSON"));

    let mut decoder = stream_decoder(None, &options(None, false));
    let outcomes: Vec<_> = decoder
        .framed(byte_chunks(&[]), Some(headers.clone()), &redactor)
        .collect();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        error(outcomes.into_iter().next().unwrap()).status(),
        Some(502)
    );

    let mut decoder = stream_decoder(None, &options(None, false));
    let mut truncated = frames[0].clone();
    truncated.truncate(truncated.len() - 2);
    let outcomes: Vec<_> = decoder
        .framed(
            byte_chunks(&[frames[0].clone(), truncated]),
            Some(headers.clone()),
            &redactor,
        )
        .collect();
    assert_eq!(outcomes.len(), 2);
    let mut outcomes = outcomes.into_iter();
    assert!(outcomes.next().unwrap().is_ok());
    assert_eq!(error(outcomes.next().unwrap()).status(), Some(502));

    let mut decoder = stream_decoder(None, &options(None, false));
    let ignored = vec![chunk_frame("")];
    let outcomes: Vec<_> = decoder
        .framed(byte_chunks(&ignored), Some(headers.clone()), &redactor)
        .collect();
    assert!(outcomes.is_empty());

    let mut decoder = stream_decoder(Some("anthropic"), &options(None, false));
    let bad_payload = vec![chunk_frame(
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
    )];
    let outcomes: Vec<_> = decoder
        .framed(byte_chunks(&bad_payload), Some(headers.clone()), &redactor)
        .collect();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(error(outcomes.into_iter().next().unwrap()).status(), None);

    let mut decoder = stream_decoder(Some("anthropic"), &options(None, false));
    let projected = vec![chunk_frame(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
    )];
    let outcomes: Vec<_> = decoder
        .framed(byte_chunks(&projected), Some(headers.clone()), &redactor)
        .collect();
    assert_eq!(
        anthropic(outcomes.into_iter().next().unwrap().unwrap()).choices[0]
            .delta
            .content,
        "hi"
    );

    let mut decoder = stream_decoder(None, &options(None, false));
    let chunks = bytes_stream(&frames);
    let outcomes = block_on(async {
        let mut stream = decoder.framed(chunks, Some(headers), &redactor);
        let mut outcomes = Vec::new();
        while let Some(item) = stream.next().await {
            outcomes.push(item);
        }
        outcomes
    });
    let texts: Vec<String> = outcomes
        .into_iter()
        .map(|outcome| generic(outcome.unwrap()).text)
        .collect();
    assert_eq!(texts, ["a", "b"]);

    assert_eq!(
        decode_bedrock_event_json(br#"{"a":1}"#).unwrap(),
        json(r#"{"a":1}"#)
    );
    assert!(decode_bedrock_event_json(b"{bad").is_err());
}

fn bytes_stream(frames: &[Vec<u8>]) -> futures_util::stream::Iter<std::vec::IntoIter<Bytes>> {
    futures_util::stream::iter(byte_chunks(frames).collect::<Vec<_>>())
}
