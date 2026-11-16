#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "b4804" => b4804, "b4805" => b4805, "b4807" => b4807, "b4808" => b4808, "b4810" => b4810,
    "b6950" => b6950, "b8202" => b8202,
}
