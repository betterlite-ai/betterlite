#![expect(
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value."
)]

betterlite_oracle_helper_api::helper_group! {
    "b10090" => b10090, "b10095" => b10095, "b10096" => b10096, "b3886" => b3886,
}
