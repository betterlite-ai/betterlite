#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "b6801" => b6801, "b6803" => b6803, "b6805" => b6805, "b6806" => b6806, "b6809" => b6809,
}
