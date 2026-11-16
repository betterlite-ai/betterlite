#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "b3240" => b3240, "b3241" => b3241, "b3242" => b3242, "b3250" => b3250, "b3252" => b3252,
    "b3253" => b3253, "b3269" => b3269,
}
