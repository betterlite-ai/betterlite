#![cfg(test)]

use super::{frame, json};
use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_agentic_stream::events::rebuild_agentic_response;

// each SSE frame as its event: name (None for a bare data: line) and its data: payload
const FIXTURE: &[(Option<&str>, &str)] = &[
    (
        Some("message_start"),
        r#"{"type":"message_start","message":{"id":"msg_9","model":"claude-x","role":"assistant","usage":{"input_tokens":5,"cache_read_input_tokens":3}}}"#,
    ),
    (
        Some("content_block_start"),
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"ignored","signature":"ignored"}}"#,
    ),
    (
        Some("content_block_delta"),
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me "}}"#,
    ),
    (
        None,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"think"}}"#,
    ),
    (
        Some(""),
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-1"}}"#,
    ),
    (
        Some("content_block_stop"),
        r#"{"type":"content_block_stop","index":0}"#,
    ),
    (
        Some("content_block_start"),
        r#"{"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"opaque"}}"#,
    ),
    (
        Some("content_block_start"),
        r#"{"type":"content_block_start","content_block":{"type":"text","text":"ignored"}}"#,
    ),
    (
        Some("content_block_delta"),
        r#"{"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"Hi"}}"#,
    ),
    (None, "not json"),
    (
        Some("content_block_delta"),
        r#"{"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":" there"}}"#,
    ),
    (
        Some("content_block_start"),
        r#"{"type":"content_block_start","index":3,"content_block":{"type":"tool_use","id":"toolu_9","name":"lookup","input":{"ignored":true}}}"#,
    ),
    (
        Some("content_block_delta"),
        r#"{"type":"content_block_delta","index":3,"delta":{"type":"input_json_delta","partial_json":"{\"q\": "}}"#,
    ),
    (
        Some("content_block_delta"),
        r#"{"type":"content_block_delta","index":3,"delta":{"type":"input_json_delta","partial_json":"1}"}}"#,
    ),
    (
        Some("content_block_stop"),
        r#"{"type":"content_block_stop","index":3}"#,
    ),
    (
        Some("content_block_start"),
        r#"{"type":"content_block_start","index":4,"content_block":{"type":"tool_use","id":"toolu_10","name":"broken"}}"#,
    ),
    (
        Some("content_block_delta"),
        r#"{"type":"content_block_delta","index":4,"delta":{"type":"input_json_delta","partial_json":"{\"q\""}}"#,
    ),
    (
        Some("content_block_stop"),
        r#"{"type":"content_block_stop","index":4}"#,
    ),
    (
        Some("content_block_start"),
        r#"{"type":"content_block_start","index":5,"content_block":{"type":"server_tool_use","id":"srvtoolu_1","name":"web_search","input":{"query":"x"}}}"#,
    ),
    (
        Some("content_block_start"),
        r#"{"type":"content_block_start","index":6,"content_block":{}}"#,
    ),
    // kept bug: the failed data line leaves message_delta pending, so it types the next data line
    (Some("message_delta"), "not json"),
    (
        None,
        r#"{"delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":12,"input_tokens":6,"cache_creation_input_tokens":2}}"#,
    ),
    (Some("message_stop"), r#"{"type":"message_stop"}"#),
];

const EXPECTED: &str = r#"{"id":"msg_9","type":"message","role":"assistant","model":"claude-x","content":[{"type":"thinking","thinking":"Let me think","signature":"sig-1"},{"type":"redacted_thinking","data":"opaque"},{"type":"text","text":"Hi there"},{"type":"tool_use","id":"toolu_9","name":"lookup","input":{"q":1}},{"type":"tool_use","id":"toolu_10","name":"broken","input":{"_raw":"{\"q\""}},{"type":"server_tool_use","id":"srvtoolu_1","name":"web_search","input":{"query":"x"}},{"type":"text","text":""}],"stop_reason":"tool_use","stop_sequence":null,"usage":{"input_tokens":6,"output_tokens":12,"cache_read_input_tokens":3,"cache_creation_input_tokens":2}}"#;

fn data_only(data: &str) -> Vec<u8> {
    format!("data: {data}\n\n").into_bytes()
}

// LiteLLM's _rebuild_anthropic_response_from_sse returns exactly EXPECTED for the same bytes
#[test]
fn rebuilds_every_block_kind() {
    let raw = FIXTURE
        .iter()
        .flat_map(|(event, data)| event.map_or_else(|| data_only(data), |name| frame(name, data)))
        .collect::<Vec<u8>>();
    let rebuilt = rebuild_agentic_response(&raw, &ApiLimits::default()).expect("the bytes rebuild");
    assert_eq!(rebuilt, Some(json(EXPECTED)));
}
