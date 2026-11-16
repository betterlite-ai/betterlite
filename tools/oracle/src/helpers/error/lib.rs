#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "anthropic_replicate_errors" => anthropic_replicate_errors, "aws_errors" => aws_errors,
    "azure_openrouter_errors" => azure_openrouter_errors, "b8410" => b8410,
    "error_call_id" => call_id, "error_class" => class, "error_facts" => facts,
    "error_normalization" => normalization, "native_error" => native_error,
    "native_errors_a" => native_errors_a, "native_errors_b" => native_errors_b,
    "openai_like_errors" => openai_like_errors, "provider_error" => provider_error,
    "vertex_errors" => vertex_errors,
}
