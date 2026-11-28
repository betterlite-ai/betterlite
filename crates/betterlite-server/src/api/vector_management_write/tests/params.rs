#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use crate::api::vector_management_write::create::params_text;

#[test]
fn params_text_stores_the_normalized_generic_params() {
    let mut raw = JsonObject::new();
    raw.insert("self".to_owned(), Json::Text("dropped".to_owned()));
    raw.insert("params".to_owned(), Json::Text("dropped".to_owned()));
    raw.insert("max_retries".to_owned(), Json::Text("3".to_owned()));
    raw.insert("use_litellm_proxy".to_owned(), Json::Bool(true));
    raw.insert("drop_params".to_owned(), Json::Text("y".to_owned()));
    raw.insert("left_out".to_owned(), Json::Null);
    let stored = params_text(Some(&raw)).expect("the params are generic");
    let dumped: JsonObject =
        serde_json::from_str(&stored).expect("the stored text is a JSON object");
    assert_eq!(None, dumped.get("self"));
    assert_eq!(None, dumped.get("params"));
    assert_eq!(
        Some(&Json::Number(Number::from(3_i64))),
        dumped.get("max_retries")
    );
    assert_eq!(Some(&Json::Bool(true)), dumped.get("use_litellm_proxy"));
    assert_eq!(Some(&Json::Text("y".to_owned())), dumped.get("drop_params"));
    assert_eq!(None, dumped.get("left_out"));
    assert_eq!(
        Some(&Json::Bool(false)),
        dumped.get("allow_client_keepalive_override")
    );
}

#[test]
fn params_text_stores_an_empty_object_for_absent_or_empty_params() {
    assert_eq!(params_text(None).expect("an empty object dumps"), "{}");
    assert_eq!(
        params_text(Some(&JsonObject::new())).expect("an empty object dumps"),
        "{}"
    );
}

#[test]
fn params_text_keeps_the_route_error_text() {
    let mut raw = JsonObject::new();
    raw.insert("max_retries".to_owned(), Json::Text("abc".to_owned()));
    let error = params_text(Some(&raw)).expect_err("abc is not a whole number");
    assert_eq!(error.to_string(), "max_retries is not a whole number: abc");
}
