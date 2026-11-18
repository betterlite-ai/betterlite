use super::typed_chunk;

use crate::stream_chunks::ChunkKind;
use crate::stream_chunks::serialize::serialize_chunk;

#[test]
fn serialize_tools_drop_web_and_file_model_nulls() {
    let chunk = typed_chunk(
        ChunkKind::Event,
        r#"{"type": "response.created", "response": {"id": "resp_tools", "created_at": 22, "output": [], "tools": [{"type": "web_search", "search_context_size": null, "filters": {"allowed_domains": null, "extra_null": null}, "user_location": {"type": "approximate", "city": null, "country": "US", "region": null, "timezone": null, "extra_null": null, "nested": {"a": null}}, "extra_null": null}, {"type": "file_search", "vector_store_ids": ["vs_1"], "max_num_results": null, "filters": {"type": "and", "filters": [{"type": "eq", "key": "k", "value": "v", "extra_null": null}, {"type": "or", "filters": [], "extra_null": null}, {"plain": true, "nested": {"a": null}}], "extra_null": null}, "ranking_options": {"ranker": null, "score_threshold": null, "hybrid_search": {"embedding_weight": 0.5, "text_weight": 0.5, "extra_null": null}, "extra_null": null}, "extra_null": null}]}}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_tools","created_at":22,"output":[],"#,
            r#""tools":[{"type":"web_search","filters":{},"user_location":{"type":"approximate","country":"US","nested":{"a":null}}},"#,
            r#"{"type":"file_search","vector_store_ids":["vs_1"],"filters":{"type":"and","filters":[{"type":"eq","key":"k","value":"v"},{"type":"or","filters":[],"extra_null":null},{"plain":true,"nested":{"a":null}}]},"#,
            r#""ranking_options":{"hybrid_search":{"embedding_weight":0.5,"text_weight":0.5}}}]}}"#
        )
    );
}

#[test]
fn serialize_tools_drop_mcp_and_container_nulls() {
    let chunk = typed_chunk(
        ChunkKind::Event,
        r#"{"type": "response.created", "response": {"id": "resp_kinds", "created_at": 23, "output": [], "tools": [{"type": "mcp", "server_label": "srv", "allowed_tools": {"read_only": null, "tool_names": null, "extra_null": null}, "require_approval": {"always": {"read_only": null, "tool_names": null, "extra_null": null}, "never": null, "extra_null": null}, "authorization": null, "headers": null, "server_url": null, "extra_null": null}, {"type": "code_interpreter", "container": {"type": "auto", "file_ids": null, "memory_limit": null, "network_policy": {"type": "allowlist", "allowed_domains": ["a.example"], "domain_secrets": [{"domain": "d.example", "name": "n", "value": "v", "extra_null": null}], "extra_null": null}, "extra_null": null}, "allowed_callers": null, "extra_null": null}]}}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_kinds","created_at":23,"output":[],"#,
            r#""tools":[{"type":"mcp","server_label":"srv","allowed_tools":{},"require_approval":{"always":{}}},"#,
            r#"{"type":"code_interpreter","container":{"type":"auto","network_policy":{"type":"allowlist","allowed_domains":["a.example"],"domain_secrets":[{"domain":"d.example","name":"n","value":"v"}]}}}]}}"#
        )
    );
}

#[test]
fn serialize_tool_calls_drop_caller_nulls() {
    let chunk = typed_chunk(
        ChunkKind::Event,
        r#"{"type": "response.created", "response": {"id": "resp_calls", "created_at": 24, "output": [], "tools": [{"type": "function_call", "arguments": "{}", "call_id": "call_1", "name": "f", "id": null, "status": null, "namespace": null, "caller": {"type": "program", "caller_id": "p", "extra_null": null}, "extra_null": null}, {"type": "function_call", "arguments": "{}", "call_id": "call_2", "name": "g", "id": null, "status": null}]}}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_calls","created_at":24,"output":[],"#,
            r#""tools":[{"type":"function_call","arguments":"{}","call_id":"call_1","name":"f","caller":{"type":"program","caller_id":"p"}},"#,
            r#"{"type":"function_call","arguments":"{}","call_id":"call_2","name":"g"}]}}"#
        )
    );
}

#[test]
fn serialize_tools_keep_unmatched_list() {
    let chunk = typed_chunk(
        ChunkKind::Event,
        r#"{"type": "response.created", "response": {"id": "resp_mixed", "created_at": 25, "output": [], "tools": [{"type": "web_search", "user_location": {"type": "approximate", "city": null, "country": "US"}, "extra_null": null}, {"type": "unknown_tool", "extra_null": null}]}}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_mixed","created_at":25,"output":[],"#,
            r#""tools":[{"type":"web_search","user_location":{"type":"approximate","city":null,"country":"US"},"extra_null":null},"#,
            r#"{"type":"unknown_tool","extra_null":null}]}}"#
        )
    );
}
