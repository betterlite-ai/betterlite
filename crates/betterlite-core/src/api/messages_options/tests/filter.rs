use rstest::rstest;

use super::super::requested_optional_params;
use super::support::object;

const EVERY_KEY: &str = r#"{
    "max_tokens": 1, "metadata": {"user_id": "u"}, "stop_sequences": ["x"], "stream": true,
    "system": "s", "temperature": 0.5, "thinking": {"type": "enabled"}, "tool_choice": {"type": "auto"},
    "tools": [{"name": "t"}], "top_k": 2, "inference_geo": "us", "top_p": 0.5, "mcp_servers": [],
    "context_management": {"edits": []}, "compaction": {"type": "summarize"}, "container": {"id": "c"},
    "output_format": {"type": "json_schema"}, "speed": "fast", "output_config": {"effort": "high"},
    "cache_control": {"type": "ephemeral"}, "reasoning_effort": "low", "safeguards": [{"a": 1}]
}"#;

const FALSY: &str =
    r#"{"stream": false, "max_tokens": 0, "tools": [], "metadata": {}, "system": ""}"#;

#[rstest]
#[case::every_declared_key(EVERY_KEY, EVERY_KEY)]
#[case::undeclared_keys(
    r#"{"messages": [], "model": "m", "litellm_metadata": {}, "unknown": 1, "max_tokens": 3}"#,
    r#"{"max_tokens": 3}"#
)]
#[case::null_values(
    r#"{"max_tokens": null, "system": null, "top_k": 4}"#,
    r#"{"top_k": 4}"#
)]
#[case::falsy_values(FALSY, FALSY)]
fn keeps_declared_non_null_keys_without_a_model(#[case] sent: &str, #[case] kept: &str) {
    let params = object(sent);
    let filtered = requested_optional_params(&params, None).expect("filtered");
    assert_eq!(filtered, object(kept));
    assert_eq!(
        filtered.keys().collect::<Vec<_>>(),
        object(kept).keys().collect::<Vec<_>>()
    );
    assert_eq!(params, object(sent));
}
