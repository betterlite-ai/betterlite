use std::collections::BTreeMap;

use betterlite_openai::embedding::EmbeddingVector;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use super::{
    EmbeddingError, get_error_class, transform_embedding_request, transform_embedding_response,
};

fn body(data: &str) -> String {
    format!(
        r#"{{"object":"list","data":{data},"model":"e5-base","usage":{{"prompt_tokens":2,"total_tokens":2}}}}"#
    )
}

#[test]
fn the_request_keeps_text_first_then_the_model_then_optional_params() {
    let input = Json::Text("hello".to_owned());
    let optional = Object::from_iter([("temperature".to_owned(), Json::Number(Number::from(0)))]);
    let request = transform_embedding_request(&input, "snowflake/e5-base", &optional);
    let keys: Vec<&str> = request.keys().map(String::as_str).collect();
    assert_eq!(keys, ["text", "model", "temperature"]);
    assert_eq!(request["text"], Json::Text("hello".to_owned()));
    // the embedding body keeps the snowflake/ prefix the chat body strips
    assert_eq!(request["model"], Json::Text("snowflake/e5-base".to_owned()));
}

#[test]
fn a_later_optional_param_overrides_the_model() {
    let input = Json::Text("hello".to_owned());
    let optional = Object::from_iter([("model".to_owned(), Json::Text("override".to_owned()))]);
    let request = transform_embedding_request(&input, "snowflake/e5-base", &optional);
    assert_eq!(request["model"], Json::Text("override".to_owned()));
}

#[test]
fn the_nested_embedding_keeps_element_zero() {
    let reply = transform_embedding_response(
        "snowflake/e5-base",
        &body(r#"[{"object":"embedding","index":0,"embedding":[[1,2,3],[4,5,6]]}]"#),
    )
    .unwrap();
    assert_eq!(reply.response.model, "snowflake/e5-base");
    assert_eq!(reply.hidden_model, Some("snowflake/e5-base".to_owned()));
    assert_eq!(
        reply.response.data[0].embedding,
        EmbeddingVector::Floats(vec![Number::from(1), Number::from(2), Number::from(3)])
    );
}

#[test]
fn a_reply_without_object_fields_gets_litellms_defaults() {
    let reply = transform_embedding_response(
        "e5-base",
        r#"{"data":[{"index":0,"embedding":[[1]]}],"model":"e5-base","usage":{"prompt_tokens":1,"total_tokens":1}}"#,
    )
    .unwrap();
    assert_eq!(reply.response.object, "list");
    assert_eq!(reply.response.data[0].object, "embedding");
}

#[test]
fn a_reply_object_field_is_kept_as_sent() {
    let reply = transform_embedding_response(
        "e5-base",
        r#"{"object":"other","data":[{"object":"vector","index":0,"embedding":[[1]]}],"model":"e5-base","usage":{"prompt_tokens":1,"total_tokens":1}}"#,
    )
    .unwrap();
    assert_eq!(reply.response.object, "other");
    assert_eq!(reply.response.data[0].object, "vector");
}

#[test]
fn a_reply_without_a_model_still_gets_the_prefix() {
    let reply = transform_embedding_response(
        "snowflake/e5-base",
        r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[[1]]}],"usage":{"prompt_tokens":1,"total_tokens":1}}"#,
    )
    .unwrap();
    assert_eq!(reply.response.model, "snowflake/");
}

#[test]
fn an_empty_embedding_array_is_an_error() {
    let error = transform_embedding_response(
        "model",
        &body(r#"[{"object":"embedding","index":0,"embedding":[]}]"#),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "the embedding at index 0 is empty");
}

#[test]
fn a_missing_embedding_array_names_the_shape() {
    let error =
        transform_embedding_response("model", &body(r#"[{"object":"embedding"}]"#)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the embedding at index 0 must be a list of numbers"
    );
}

#[test]
fn a_non_number_first_element_names_the_shape() {
    let error = transform_embedding_response(
        "model",
        &body(r#"[{"object":"embedding","index":0,"embedding":[["x"]]}]"#),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the embedding at index 0 must be a list of numbers"
    );
}

#[test]
fn a_non_object_item_names_its_index() {
    let error = transform_embedding_response("model", &body("[5]")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the embedding at index 0 must be an object"
    );
}

#[test]
fn data_that_is_not_a_list_is_an_error() {
    let error = transform_embedding_response(
        "model",
        r#"{"object":"list","data":{},"usage":{"prompt_tokens":1,"total_tokens":1}}"#,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "the embedding reply data must be a list");
}

#[test]
fn an_unreadable_reply_is_a_decode_error() {
    assert!(matches!(
        transform_embedding_response("model", "not json"),
        Err(EmbeddingError::Decode { .. })
    ));
}

#[test]
fn the_error_class_carries_the_status_message_and_headers() {
    let headers = BTreeMap::from_iter([("x-request-id".to_owned(), "1".to_owned())]);
    let error = get_error_class("upstream exploded", 429, headers.clone());
    assert_eq!(error.message, "upstream exploded");
    assert_eq!(error.status_code, 429);
    assert_eq!(error.headers, headers);
}
