#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "b3662" => b3662, "b3691" => b3691, "b3900" => b3900, "b4681" => b4681, "b5630" => b5630,
    "jwt_settings" => jwt_settings, "redis_credentials" => redis_credentials,
}
