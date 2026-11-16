#![expect(
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value."
)]
#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        reason = "The tests build provider JSON with serde_json::json!."
    )
)]

betterlite_oracle_helper_api::helper_group! {
    "b9963" => b9963,
    "image_tokens" => image_tokens,
    "media_tokens" => media_tokens,
    "reservation_tokens" => reservation_tokens,
    "token_count" => token_count,
    "token_formatting" => token_formatting,
    "tokenizer_engine" => engine,
}
