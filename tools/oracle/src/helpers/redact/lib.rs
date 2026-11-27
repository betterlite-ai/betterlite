#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "b10137" => b10137,
    "redaction" => redaction,
    "sentry_scrubbing" => sentry_scrubbing,
    "structured_redaction" => structured_redaction,
}
