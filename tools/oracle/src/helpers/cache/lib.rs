#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "b4290" => b4290, "b4291" => b4291, "b4293" => b4293, "b4294" => b4294, "b4296" => b4296,
    "b4297" => b4297, "b4298" => b4298, "b4300" => b4300, "b4304" => b4304,
}
