#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "b3461" => b3461, "b5367" => b5367, "b5368" => b5368, "b5650" => b5650,
    "proxy_auth" => proxy_auth,
}
