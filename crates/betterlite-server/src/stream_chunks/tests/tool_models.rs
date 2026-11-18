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
fn serialize_tools_shape_known_type_with_bad_values() {
    let chunk = typed_chunk(
        ChunkKind::Event,
        r#"{"type": "response.created", "response": {"id": "resp_bad", "created_at": 26, "output": [], "tools": [{"type": "web_search", "filters": {"allowed_domains": "nope", "extra_null": null}, "extra_null": null}]}}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_bad","created_at":26,"output":[],"#,
            r#""tools":[{"type":"web_search","filters":{"allowed_domains":"nope"}}]}}"#
        )
    );
}

#[test]
fn serialize_tools_drop_image_mask_nulls() {
    let chunk = typed_chunk(
        ChunkKind::Event,
        r#"{"type": "response.created", "response": {"id": "resp_mask", "created_at": 27, "output": [], "tools": [{"type": "image_generation", "size": null, "input_image_mask": {"image_url": null, "file_id": "file_1", "extra_null": null, "nested": {"a": null}}, "extra_null": null}]}}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_mask","created_at":27,"output":[],"#,
            r#""tools":[{"type":"image_generation","input_image_mask":{"file_id":"file_1","nested":{"a":null}}}]}}"#
        )
    );
}

#[test]
fn serialize_tools_drop_custom_format_nulls() {
    let chunk = typed_chunk(
        ChunkKind::Event,
        r#"{"type": "response.created", "response": {"id": "resp_custom", "created_at": 28, "output": [], "tools": [{"type": "custom", "name": "c", "description": null, "format": {"type": "grammar", "definition": "d", "syntax": null, "extra_null": null}, "extra_null": null}, {"type": "custom", "name": "c2", "format": {"type": "text", "extra_null": null}}]}}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_custom","created_at":28,"output":[],"#,
            r#""tools":[{"type":"custom","name":"c","format":{"type":"grammar","definition":"d"}},"#,
            r#"{"type":"custom","name":"c2","format":{"type":"text"}}]}}"#
        )
    );
}

#[test]
fn serialize_tools_drop_namespace_tool_nulls() {
    let chunk = typed_chunk(
        ChunkKind::Event,
        r#"{"type": "response.created", "response": {"id": "resp_namespace", "created_at": 29, "output": [], "tools": [{"type": "namespace", "name": "n", "description": null, "tools": [{"type": "function", "name": "f", "description": null, "strict": null, "parameters": {"a": null}, "extra_null": null}, {"type": "custom", "name": "c2", "format": {"type": "text", "extra_null": null}}], "extra_null": null}]}}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_namespace","created_at":29,"output":[],"#,
            r#""tools":[{"type":"namespace","name":"n","tools":[{"type":"function","name":"f","parameters":{"a":null}},"#,
            r#"{"type":"custom","name":"c2","format":{"type":"text"}}]}]}}"#
        )
    );
}

#[test]
fn serialize_tools_drop_shell_skill_nulls() {
    let chunk = typed_chunk(
        ChunkKind::Event,
        r#"{"type": "response.created", "response": {"id": "resp_shell", "created_at": 30, "output": [], "tools": [{"type": "shell", "name": "s", "environment": {"type": "container_auto", "file_ids": null, "network_policy": {"type": "disabled", "extra_null": null}, "skills": [{"type": "skill_reference", "skill_id": "sk", "version": null, "extra_null": null}, {"type": "inline", "name": "i", "description": null, "source": {"type": "base64", "data": "eA==", "media_type": null, "extra_null": null}, "extra_null": null}], "extra_null": null}, "extra_null": null}]}}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"type":"response.created","response":{"id":"resp_shell","created_at":30,"output":[],"#,
            r#""tools":[{"type":"shell","name":"s","environment":{"type":"container_auto","network_policy":{"type":"disabled"},"#,
            r#""skills":[{"type":"skill_reference","skill_id":"sk"},{"type":"inline","name":"i","source":{"type":"base64","data":"eA=="}}]}}]}}"#
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
