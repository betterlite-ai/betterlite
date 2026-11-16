#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "b4414" => b4414, "b4415" => b4415, "b4416" => b4416, "b4417" => b4417, "b4418" => b4418,
    "b4420" => b4420, "b8500" => b8500,
}
