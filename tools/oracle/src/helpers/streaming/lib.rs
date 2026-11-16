#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "b5652" => b5652, "b5653" => b5653, "stream_base_assembly" => stream_base_assembly,
    "stream_content_assembly" => stream_content_assembly,
    "stream_tool_assembly" => stream_tool_assembly,
    "stream_usage_assembly" => stream_usage_assembly,
    "streamed_response_assembly" => streamed_response_assembly,
}
