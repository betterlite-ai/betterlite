#![expect(
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value."
)]

betterlite_oracle_helper_api::helper_group! {
    "b4502" => b4502, "b4507" => b4507, "lens_state" => state,
}
