use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_openai::chat_choices::wire::StreamReply;
use betterlite_streaming::stream_chunk_shaping::records::{GenericChunk, JsonValue};
use bytes::Bytes;
use futures_util::stream;

use crate::converse_events::ChunkProjection;
use crate::converse_events::response::ConverseChunk;
use crate::invoke_events::error::InvokeStreamError;
use crate::invoke_events::fake::{FakeResponse, FakeStream};
use crate::invoke_events::framing::decode_bedrock_event_json;
use crate::invoke_events::{
    InvokeChunk, InvokeStreamDecoder, bedrock_event_stream_decoder, stream_decoder,
};

use super::support::{
    NOW, block_on, chunk_frame, fixed_id, json, options, redactor, request_headers,
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

fn first_chunk(source: &str, json_mode: Option<bool>) -> GenericChunk {
    fake(source, json_mode).into_iter().next().unwrap().unwrap()
}

fn json_value(source: &str) -> JsonValue {
    serde_json::from_str(source).unwrap()
}

fn byte_chunks(frames: &[Vec<u8>]) -> impl Iterator<Item = Bytes> + '_ {
    frames.iter().cloned().map(Bytes::from)
}

pub(super) fn claude3_invoke_stream() {
    let mut decoder = stream_decoder(Some("anthropic"), &options(Some(false), false));
    tool_use_start(&mut decoder);
    text_and_thinking_deltas(&mut decoder);
    usage_and_stop(&mut decoder);
    native_error_status(&mut decoder);
    json_mode_suppression();
}

fn tool_use_start(decoder: &mut InvokeStreamDecoder) {
    let start = anthropic(parse(
        decoder,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"f","input":{}}}"#,
    ));
    let call = &start.choices[0].delta.tool_calls.as_ref().unwrap()[0];
    assert_eq!(call.id.as_deref(), Some("t1"));
    assert_eq!(call.function.name.as_deref(), Some("f"));
    assert_eq!(call.function.arguments, "");
}

fn text_and_thinking_deltas(decoder: &mut InvokeStreamDecoder) {
    let text = anthropic(parse(
        decoder,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
    ));
    assert_eq!(text.choices[0].delta.content, "hi");
    assert_eq!(text.id, format!("chatcmpl-{}", fixed_id()));
    let thinking = anthropic(parse(
        decoder,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"plan"}}"#,
    ));
    assert_eq!(
        thinking.choices[0].delta.reasoning_content.as_deref(),
        Some("plan")
    );
    let signature = anthropic(parse(
        decoder,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}"#,
    ));
    assert!(signature.choices[0].delta.thinking_blocks.is_some());
}

fn usage_and_stop(decoder: &mut InvokeStreamDecoder) {
    let usage_start = anthropic(parse(
        decoder,
        r#"{"type":"message_start","message":{"model":"claude","usage":{"input_tokens":1,"output_tokens":2}}}"#,
    ));
    assert!(usage_start.usage.is_some());
    assert_eq!(usage_start.model.as_deref(), Some("claude"));
    let finished = anthropic(parse(
        decoder,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#,
    ));
    assert_eq!(finished.choices[0].finish_reason.as_deref(), Some("stop"));
    // the deviation in docs/deviations/B609.yaml: the streamed thinking text is not tokenized
    let details = finished.usage.unwrap().completion_tokens_details;
    assert_eq!(details.reasoning, Some(0));
}

// LiteLLM raises AnthropicError(status_code=500, message=...) for a native stream error event.
fn native_error_status(decoder: &mut InvokeStreamDecoder) {
    let failure =
        error(decoder.chunk_parser(&json(r#"{"type":"error","error":{"message":"boom"}}"#)));
    assert_eq!(failure.status(), Some(500));
    let malformed = error(decoder.chunk_parser(&json(r#"{"type":"content_block_start"}"#)));
    assert_eq!(malformed.status(), None);
}

fn json_mode_suppression() {
    let mut converted = stream_decoder(Some("anthropic"), &options(Some(true), true));
    let suppressed = anthropic(parse(
        &mut converted,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"j","name":"json_tool_call","input":{}}}"#,
    ));
    assert!(suppressed.choices[0].delta.tool_calls.is_none());
    assert_eq!(suppressed.choices[0].delta.content, "");
}

pub(super) fn json_mode_tool_chunk() {
    json_mode_values();
    json_mode_null_values();
    json_mode_whole_object();
    json_mode_malformed_arguments();
    json_mode_missing_arguments();
    plain_tool_call();
    plain_missing_arguments();
    missing_id();
    missing_function();
    dict_arguments();
    empty_tool_list();
}

fn json_mode_values() {
    let chunk = first_chunk(
        r#"{"choices":[{"message":{"content":"raw","tool_calls":[{"id":"c1","type":"function","function":{"name":"json_tool_call","arguments":"{\"values\": {\"a\": 1}}"}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    assert_eq!(chunk.text, r#"{"a": 1}"#);
    assert!(chunk.tool_use.is_none());
}

// _convert_tool_response_to_message dumps the whole argument object when values is present but null.
fn json_mode_null_values() {
    let chunk = first_chunk(
        r#"{"choices":[{"message":{"content":"raw","tool_calls":[{"id":"c1","type":"function","function":{"name":"json_tool_call","arguments":"{\"values\": null}"}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    assert_eq!(chunk.text, r#"{"values": null}"#);
    assert!(chunk.tool_use.is_none());
}

fn json_mode_whole_object() {
    let chunk = first_chunk(
        r#"{"choices":[{"message":{"content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"json_tool_call","arguments":"{\"a\": 1}"}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    assert_eq!(chunk.text, r#"{"a": 1}"#);
    assert!(chunk.tool_use.is_none());
}

fn json_mode_malformed_arguments() {
    let chunk = first_chunk(
        r#"{"choices":[{"message":{"content":"kept","tool_calls":[{"id":"c1","type":"function","function":{"name":"json_tool_call","arguments":"not json"}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    assert_eq!(chunk.text, "not json");
    assert!(chunk.tool_use.is_none());
}

fn json_mode_missing_arguments() {
    let chunk = first_chunk(
        r#"{"choices":[{"message":{"content":"kept","tool_calls":[{"id":"c1","type":"function","function":{"name":"json_tool_call"}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    assert_eq!(chunk.text, "");
    assert!(chunk.tool_use.is_none());
}

fn plain_missing_arguments() {
    let chunk = first_chunk(
        r#"{"choices":[{"message":{"content":"kept","tool_calls":[{"id":"c1","type":"function","function":{"name":"f"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(false),
    );
    assert_eq!(
        chunk.tool_use,
        Some(json_value(
            r#"{"id":"c1","type":"function","function":{"name":"f","arguments":""},"index":0}"#
        ))
    );
}

// docs/deviations/B609.yaml: LiteLLM's ChatCompletionMessageToolCall invents a uuid4 when the id is absent.
fn missing_id() {
    let chunk = first_chunk(
        r#"{"choices":[{"message":{"content":"","tool_calls":[{"function":{"name":"f","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(false),
    );
    assert_eq!(
        chunk.tool_use,
        Some(json_value(
            r#"{"id":null,"type":"function","function":{"name":"f","arguments":"{}"},"index":0}"#
        ))
    );
}

// docs/deviations/B609.yaml: LiteLLM's ChatCompletionMessageToolCall requires function and refuses the response.
fn missing_function() {
    let chunk = first_chunk(
        r#"{"choices":[{"message":{"content":"raw","tool_calls":[{"id":"c1"}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(false),
    );
    assert_eq!(
        chunk.tool_use,
        Some(json_value(
            r#"{"id":"c1","type":"function","function":{"name":null,"arguments":""},"index":0}"#
        ))
    );
}

fn plain_tool_call() {
    let chunk = first_chunk(
        r#"{"choices":[{"message":{"content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}},{"id":"c2","type":"function","function":{"name":"g","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(false),
    );
    assert_eq!(
        chunk.tool_use,
        Some(json_value(
            r#"{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"},"index":0}"#
        ))
    );
}

fn dict_arguments() {
    let converted = first_chunk(
        r#"{"choices":[{"message":{"content":"raw","tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":{"values":{"a":1}}}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    assert_eq!(converted.text, r#"{"a": 1}"#);
    assert!(converted.tool_use.is_none());

    let plain = first_chunk(
        r#"{"choices":[{"message":{"content":"raw","tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":{"a":1,"b":[2,3]}}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(false),
    );
    assert_eq!(
        plain.tool_use,
        Some(json_value(
            r#"{"id":"c1","type":"function","function":{"name":"f","arguments":"{\"a\": 1, \"b\": [2, 3]}"},"index":0}"#
        ))
    );
}

fn empty_tool_list() {
    let real = first_chunk(
        r#"{"choices":[{"message":{"content":"x","tool_calls":[]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(false),
    );
    assert_eq!(real.text, "x");
    assert!(real.tool_use.is_none());

    let converted = first_chunk(
        r#"{"choices":[{"message":{"content":"x","tool_calls":[]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(true),
    );
    assert_eq!(converted.text, "x");
    assert!(converted.tool_use.is_none());
}

pub(super) fn json_mode_stop() {
    finished_chunk();
    finish_reason_defaults();
}

fn finished_chunk() {
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
}

fn finish_reason_defaults() {
    let mut async_stream = fake(
        r#"{"choices":[{"message":{"content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        None,
    );
    let chunk = block_on(futures_util::StreamExt::next(&mut async_stream))
        .unwrap()
        .unwrap();
    assert_eq!(chunk.finish_reason, "stop");
    assert!(block_on(futures_util::StreamExt::next(&mut async_stream)).is_none());

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
    let chunk = first_chunk(
        r#"{"choices":[{"message":{"content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":""}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        Some(false),
    );
    assert_eq!(
        chunk.tool_use,
        Some(json_value(
            r#"{"id":"c1","type":"function","function":{"name":"f","arguments":""},"index":0}"#
        ))
    );

    let mut missing_usage = fake(
        r#"{"choices":[{"message":{"content":"x"},"finish_reason":"stop"}]}"#,
        None,
    );
    let error = missing_usage.next().unwrap().unwrap_err();
    assert_eq!(error.status(), None);
    assert!(error.to_string().starts_with("Failed to decode chunk:"));
    assert!(missing_usage.next().is_none());

    let mut async_missing_usage = fake(
        r#"{"choices":[{"message":{"content":"x"},"finish_reason":"stop"}]}"#,
        None,
    );
    let error = block_on(futures_util::StreamExt::next(&mut async_missing_usage))
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().starts_with("Failed to decode chunk:"));
    assert!(block_on(futures_util::StreamExt::next(&mut async_missing_usage)).is_none());

    let missing_choices = fake(
        r#"{"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
        None,
    );
    let error = missing_choices.into_iter().next().unwrap().unwrap_err();
    assert!(error.to_string().starts_with("Failed to decode chunk:"));
}

pub(super) fn deepseek_stream_chunks() {
    let mut decoder = stream_decoder(Some("deepseek_r1"), &options(None, false));
    thinking_chunk(&mut decoder);
    transition_chunk(&mut decoder);
    answer_chunk(&mut decoder);
    mapped_finish_reasons();
    literal_marker(&mut decoder);
    partial_marker();
    token_totals();
    missing_fields();
}

fn thinking_chunk(decoder: &mut InvokeStreamDecoder) {
    let thinking = reply(parse(
        decoder,
        r#"{"generation":"thinking","stop_reason":"stop","prompt_token_count":1,"generation_token_count":2}"#,
    ));
    let delta = &thinking.choices[0].delta;
    assert_eq!(delta.reasoning_content.as_deref(), Some("thinking"));
    assert_eq!(delta.content, None);
    assert_eq!(delta.role, None);
    assert_eq!(thinking.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        thinking.usage,
        Some(serde_json::json!({"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}))
    );
    assert_eq!(thinking.id, format!("chatcmpl-{}", fixed_id()));
    assert_eq!(thinking.created, NOW);
    assert_eq!(thinking.object, "chat.completion.chunk");
    assert_eq!(thinking.model, None);
}

fn transition_chunk(decoder: &mut InvokeStreamDecoder) {
    let transition = reply(parse(
        decoder,
        r#"{"generation":"</think>","stop_reason":"stop","prompt_token_count":0,"generation_token_count":0}"#,
    ));
    assert_eq!(transition.choices[0].delta.content.as_deref(), Some(""));
    assert_eq!(transition.choices[0].delta.reasoning_content, None);
}

fn answer_chunk(decoder: &mut InvokeStreamDecoder) {
    let answer = reply(parse(
        decoder,
        r#"{"generation":"answer","stop_reason":"max_tokens","prompt_token_count":0,"generation_token_count":0}"#,
    ));
    assert_eq!(answer.choices[0].delta.content.as_deref(), Some("answer"));
    assert_eq!(answer.choices[0].delta.reasoning_content, None);
    assert_eq!(answer.choices[0].finish_reason.as_deref(), Some("length"));
}

// StreamingChoices.__init__ maps a non-empty stop_reason through map_finish_reason and keeps None for empty.
fn mapped_finish_reasons() {
    let mut decoder = stream_decoder(Some("deepseek_r1"), &options(None, false));
    let unknown = reply(parse(
        &mut decoder,
        r#"{"generation":"x","stop_reason":"something_new","prompt_token_count":0,"generation_token_count":0}"#,
    ));
    assert_eq!(unknown.choices[0].finish_reason.as_deref(), Some("stop"));
    let guarded = reply(parse(
        &mut decoder,
        r#"{"generation":"x","stop_reason":"guardrail_intervened","prompt_token_count":0,"generation_token_count":0}"#,
    ));
    assert_eq!(
        guarded.choices[0].finish_reason.as_deref(),
        Some("content_filter")
    );
    let empty = reply(parse(
        &mut decoder,
        r#"{"generation":"x","stop_reason":"","prompt_token_count":0,"generation_token_count":0}"#,
    ));
    assert_eq!(empty.choices[0].finish_reason, None);
}

fn literal_marker(decoder: &mut InvokeStreamDecoder) {
    let literal = reply(parse(
        decoder,
        r#"{"generation":"</think>","stop_reason":null,"prompt_token_count":0,"generation_token_count":0}"#,
    ));
    assert_eq!(
        literal.choices[0].delta.content.as_deref(),
        Some("</think>")
    );
    assert_eq!(literal.choices[0].finish_reason, None);
}

fn partial_marker() {
    let mut decoder = stream_decoder(Some("deepseek_r1"), &options(None, false));
    let partial = reply(parse(
        &mut decoder,
        r#"{"generation":"a</think>","stop_reason":"stop","prompt_token_count":0,"generation_token_count":0}"#,
    ));
    assert_eq!(
        partial.choices[0].delta.reasoning_content.as_deref(),
        Some("a</think>")
    );
    assert_eq!(partial.choices[0].delta.content, None);
}

fn token_totals() {
    let mut decoder = stream_decoder(Some("deepseek_r1"), &options(None, false));
    let counted = reply(parse(
        &mut decoder,
        r#"{"generation":"x","stop_reason":"stop","prompt_token_count":-1,"generation_token_count":2}"#,
    ));
    assert_eq!(
        counted.usage,
        Some(serde_json::json!({"prompt_tokens":-1,"completion_tokens":2,"total_tokens":1}))
    );
    let null_counts = reply(parse(
        &mut decoder,
        r#"{"generation":"x","stop_reason":"stop","prompt_token_count":null,"generation_token_count":null}"#,
    ));
    assert_eq!(
        null_counts.usage,
        Some(serde_json::json!({"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}))
    );
}

fn missing_fields() {
    let mut decoder = stream_decoder(Some("deepseek_r1"), &options(None, false));
    assert!(
        decoder
            .chunk_parser(&json(
                r#"{"stop_reason":"stop","prompt_token_count":0,"generation_token_count":0}"#
            ))
            .is_err()
    );
    assert!(
        decoder
            .chunk_parser(&json(
                r#"{"generation":"x","prompt_token_count":0,"generation_token_count":0}"#
            ))
            .is_err()
    );
    assert!(
        decoder
            .chunk_parser(&json(r#"{"generation":"x","stop_reason":"stop"}"#))
            .is_ok()
    );
}

pub(super) fn stream_selector_variants() {
    generic_selector();
    converse_selector();
    moonshot_selector();
    deepseek_selector();
    anthropic_selector();
}

fn generic_selector() {
    let probe = json(
        r#"{"generation":"x","stop_reason":"stop","prompt_token_count":0,"generation_token_count":0}"#,
    );
    let mut explicit = stream_decoder(None, &options(Some(true), false));
    assert_eq!(generic(explicit.chunk_parser(&probe).unwrap()).text, "x");
    let mut legacy = bedrock_event_stream_decoder(None, &options(Some(true), false));
    assert_eq!(generic(legacy.chunk_parser(&probe).unwrap()).text, "x");
}

fn converse_selector() {
    let converse_probe = json(
        r#"{"start":{"toolUse":{"toolUseId":"j","name":"json_tool_call"}},"contentBlockIndex":0}"#,
    );
    let mut explicit_json = stream_decoder(None, &options(Some(true), false));
    let suppressed = converse(explicit_json.chunk_parser(&converse_probe).unwrap());
    assert!(suppressed.delta.tool_calls.is_none());
    assert_eq!(suppressed.id, format!("chatcmpl-{}", fixed_id()));
    let mut legacy_json = bedrock_event_stream_decoder(None, &options(Some(true), false));
    assert!(
        converse(legacy_json.chunk_parser(&converse_probe).unwrap())
            .delta
            .tool_calls
            .is_some()
    );
}

fn moonshot_selector() {
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
}

fn deepseek_selector() {
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
}

fn anthropic_selector() {
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

    let json_tool = json(
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"j","name":"json_tool_call","input":{}}}"#,
    );
    let mut legacy_json_mode =
        bedrock_event_stream_decoder(Some("anthropic"), &options(Some(true), true));
    let suppressed = anthropic(legacy_json_mode.chunk_parser(&json_tool).unwrap());
    assert!(suppressed.choices[0].delta.tool_calls.is_none());
    assert_eq!(suppressed.choices[0].delta.content, "");
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
    let usage = chunk.usage.as_ref().unwrap();
    assert_eq!(usage["prompt_tokens"], 1);
    assert_eq!(usage["completion_tokens"], 2);
    assert_eq!(usage["total_tokens"], 3);

    let generated = reply(parse(
        &mut decoder,
        r#"{"choices":[{"index":0,"delta":{"content":"x"}}]}"#,
    ));
    assert_eq!(generated.id, format!("chatcmpl-{}", fixed_id()));
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
    frames_and_split();
    frame_errors();
    anthropic_frames();
    async_and_json();
}

fn frame_outcomes(
    provider: Option<&str>,
    frames: &[Vec<u8>],
) -> Vec<Result<InvokeChunk, InvokeStreamError>> {
    let redactor = redactor();
    let headers = request_headers();
    let mut decoder = stream_decoder(provider, &options(None, false));
    decoder
        .framed(byte_chunks(frames), Some(headers), &redactor)
        .collect()
}

fn frame_texts(frames: &[Vec<u8>]) -> Vec<String> {
    frame_outcomes(None, frames)
        .into_iter()
        .map(|outcome| generic(outcome.unwrap()).text)
        .collect()
}

fn frames_and_split() {
    let frames = vec![
        chunk_frame(r#"{"outputText":"a"}"#),
        chunk_frame(r#"{"outputText":"b"}"#),
    ];
    assert_eq!(frame_texts(&frames), ["a", "b"]);

    let mut combined = chunk_frame(r#"{"outputText":"a"}"#);
    combined.extend(chunk_frame(r#"{"outputText":"b"}"#));
    assert_eq!(frame_texts(&[combined]), ["a", "b"]);

    let mut split = frames[0].clone();
    let tail = split.split_off(split.len().saturating_sub(3));
    assert_eq!(frame_texts(&[split, tail]), ["a"]);

    let outcomes = frame_outcomes(None, &[chunk_frame(""), chunk_frame("{}")]);
    assert_eq!(outcomes.len(), 1);
    let only = generic(outcomes.into_iter().next().unwrap().unwrap());
    assert_eq!(only.text, "");
    assert!(!only.is_finished);
}

fn frame_errors() {
    let outcomes = frame_outcomes(
        None,
        &[
            chunk_frame(r#"{"outputText":"first"}"#),
            chunk_frame("{not json"),
            chunk_frame(r#"{"outputText":"never"}"#),
        ],
    );
    assert_eq!(outcomes.len(), 2);
    let mut remaining = outcomes.into_iter();
    assert_eq!(generic(remaining.next().unwrap().unwrap()).text, "first");
    let failure = error(remaining.next().unwrap());
    assert_eq!(failure.status(), None);
    assert!(failure.to_string().contains("not valid JSON"));

    let outcomes = frame_outcomes(None, &[]);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        error(outcomes.into_iter().next().unwrap()).status(),
        Some(502)
    );

    let frame = chunk_frame(r#"{"outputText":"a"}"#);
    let mut truncated = frame.clone();
    truncated.truncate(truncated.len().saturating_sub(2));
    let outcomes = frame_outcomes(None, &[frame, truncated]);
    assert_eq!(outcomes.len(), 2);
    let mut remaining = outcomes.into_iter();
    assert!(remaining.next().unwrap().is_ok());
    assert_eq!(error(remaining.next().unwrap()).status(), Some(502));
}

fn anthropic_frames() {
    assert!(frame_outcomes(None, &[chunk_frame("")]).is_empty());

    let bad_payload = vec![chunk_frame(
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
    )];
    let outcomes = frame_outcomes(Some("anthropic"), &bad_payload);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(error(outcomes.into_iter().next().unwrap()).status(), None);

    let projected = vec![chunk_frame(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
    )];
    let outcomes = frame_outcomes(Some("anthropic"), &projected);
    assert_eq!(
        anthropic(outcomes.into_iter().next().unwrap().unwrap()).choices[0]
            .delta
            .content,
        "hi"
    );
}

fn async_and_json() {
    let redactor = redactor();
    let headers = request_headers();
    let frames = vec![
        chunk_frame(r#"{"outputText":"a"}"#),
        chunk_frame(r#"{"outputText":"b"}"#),
    ];
    let mut decoder = stream_decoder(None, &options(None, false));
    let chunks = stream::iter(byte_chunks(&frames));
    let outcomes = block_on(async {
        let mut stream = decoder.framed(chunks, Some(headers), &redactor);
        let mut outcomes = Vec::new();
        while let Some(item) = futures_util::StreamExt::next(&mut stream).await {
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
