#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use crate::api::vector_management_write::params::generic_params_dump;

#[test]
fn generic_params_dump_drops_reserved_keys_and_fills_the_false_defaults() {
    let mut raw = JsonObject::new();
    raw.insert("self".to_owned(), Json::Text("dropped".to_owned()));
    raw.insert("params".to_owned(), Json::Text("dropped".to_owned()));
    raw.insert("max_retries".to_owned(), Json::Text("3".to_owned()));
    raw.insert("use_litellm_proxy".to_owned(), Json::Bool(true));
    raw.insert("left_out".to_owned(), Json::Null);
    let dumped = generic_params_dump(&raw).expect("the params are generic");
    assert_eq!(None, dumped.get("self"));
    assert_eq!(None, dumped.get("params"));
    assert_eq!(
        Some(&Json::Number(Number::from(3_i64))),
        dumped.get("max_retries")
    );
    assert_eq!(Some(&Json::Bool(true)), dumped.get("use_litellm_proxy"));
    assert_eq!(None, dumped.get("left_out"));
    assert_eq!(
        Some(&Json::Bool(false)),
        dumped.get("allow_client_keepalive_override")
    );
}
