#![cfg(test)]

use super::{EmbeddingReply, embedding_body, voyage_body, voyage_reply};
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::Object as Body;
use serde_json::json;

#[test]
fn selects_all_embedding_request_transforms() {
    let input = EmbeddingInput::Text(String::from("hello"));
    let empty = Body::new();
    let voyage = embedding_body("voyage-3", &input, &empty).expect("voyage request");
    assert_eq!(
        voyage.get("model"),
        Some(&super::Json::Text(String::from("voyage-3")))
    );
    assert!(voyage.contains_key("input"));
    let cohere = embedding_body("cohere/embed-english", &input, &empty).expect("cohere request");
    assert!(cohere.contains_key("texts"));
    let hf = embedding_body("hf-model", &input, &empty).expect("HF request");
    assert!(hf.contains_key("inputs"));
}

#[test]
fn voyage_request_copies_caller_options_after_default_fields() {
    let input = EmbeddingInput::Text(String::from("hello"));
    let options: Body =
        serde_json::from_value(json!({"input": "override", "extra": true})).expect("options");
    let body = voyage_body("voyage-3", &input, &options);
    assert_eq!(
        body.get("input"),
        Some(&super::Json::Text(String::from("override")))
    );
    assert_eq!(
        body.get("model"),
        Some(&super::Json::Text(String::from("voyage-3")))
    );
    assert_eq!(body.get("extra"), Some(&super::Json::Bool(true)));
}

#[test]
fn voyage_reply_projects_json_and_reports_invalid_json() {
    let reply = voyage_reply(br#"{"data": [{"embedding": [1, 2]}]}"#).expect("reply");
    assert!(matches!(reply, EmbeddingReply::Voyage(_)));
    let error = voyage_reply(b"not-json").expect_err("invalid reply");
    assert_eq!(error.status_code, 200);
    assert_eq!(error.message, "not-json");
}

#[test]
fn voyage_reply_returns_projected_value() {
    assert!(matches!(
        voyage_reply(br#"{"data": []}"#).expect("reply"),
        EmbeddingReply::Voyage(_)
    ));
}
