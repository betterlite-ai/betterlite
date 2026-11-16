use betterlite_openai::embedding::EmbeddingVector;
use rstest::rstest;

use crate::embeddings::error::FoundryEmbeddingError;
use crate::embeddings::reply::{FoundryEmbeddingResponse, decode_foundry_embedding};

#[test]
fn decodes_every_reply_field() {
    let body = r#"{"object":"list","model":"embed-model","data":[{"object":"embedding","index":2,"embedding":[0.5,0.25]}],"usage":{"prompt_tokens":4,"completion_tokens":1,"total_tokens":5}}"#;
    let reply = decode_foundry_embedding(body.as_bytes()).unwrap();
    assert_eq!(reply.object.as_deref(), Some("list"));
    assert_eq!(reply.model, Some(Some("embed-model".to_owned())));
    let data = reply.data.unwrap().unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0].index, 2);
    assert!(matches!(
        data[0].embedding,
        EmbeddingVector::Floats(ref values) if values.len() == 2
    ));
    let usage = reply.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 4);
    assert_eq!(usage.completion_tokens, 1);
    assert_eq!(usage.total_tokens, 5);
    assert!(usage.completion_tokens_details.is_none());
    assert!(usage.prompt_tokens_details.is_none());
    assert!(reply.additional_headers.is_empty());
}

#[test]
fn decodes_base64_embeddings() {
    let body = r#"{"data":[{"object":"embedding","index":0,"embedding":"AAAA"}]}"#;
    let reply = decode_foundry_embedding(body.as_bytes()).unwrap();
    let data = reply.data.unwrap().unwrap();
    assert!(matches!(
        data[0].embedding,
        EmbeddingVector::Base64(ref encoded) if encoded == "AAAA"
    ));
}

#[test]
fn keeps_absent_and_null_distinct() {
    let empty = decode_foundry_embedding(b"{}").unwrap();
    assert!(empty.object.is_none());
    assert!(empty.model.is_none());
    assert!(empty.data.is_none());
    assert!(empty.usage.is_none());
    let nulls = decode_foundry_embedding(br#"{"model":null,"data":null,"usage":null}"#).unwrap();
    assert_eq!(nulls.model, Some(None));
    assert_eq!(nulls.data, Some(None));
    assert!(nulls.usage.is_none());
    let named = decode_foundry_embedding(br#"{"model":"named"}"#).unwrap();
    assert_eq!(named.model, Some(Some("named".to_owned())));
}

#[test]
fn defaults_usage_counts() {
    let empty = decode_foundry_embedding(br#"{"usage":{}}"#).unwrap();
    let zero = empty.usage.unwrap();
    assert_eq!(zero.prompt_tokens, 0);
    assert_eq!(zero.completion_tokens, 0);
    assert_eq!(zero.total_tokens, 0);
    let partial =
        decode_foundry_embedding(br#"{"usage":{"prompt_tokens":3,"total_tokens":9}}"#).unwrap();
    let counted = partial.usage.unwrap();
    assert_eq!(counted.prompt_tokens, 3);
    assert_eq!(counted.completion_tokens, 0);
    assert_eq!(counted.total_tokens, 9);
}

#[rstest]
#[case::top_level_null("null")]
#[case::top_level_array("[]")]
#[case::top_level_string("\"text\"")]
#[case::data_an_object("{\"data\":{}}")]
#[case::entry_without_fields("{\"data\":[{}]}")]
fn rejects_wrong_shapes(#[case] body: &str) {
    let error = decode_foundry_embedding(body.as_bytes()).unwrap_err();
    assert!(
        matches!(error, FoundryEmbeddingError::Json { .. }),
        "{body}"
    );
}

#[test]
fn rejects_a_null_object() {
    let null_object = decode_foundry_embedding(br#"{"object":null,"data":[]}"#).unwrap_err();
    assert!(matches!(
        null_object,
        FoundryEmbeddingError::FieldType {
            field: "object",
            ..
        }
    ));
    assert_eq!(
        null_object.to_string(),
        "the provider answer field object must be a string"
    );
}

#[test]
fn apply_reply_copies_counts_only() {
    let mut response = FoundryEmbeddingResponse::default();
    assert_eq!(response.object, "list");
    assert!(response.model.is_none());
    assert!(response.data.is_empty());
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.completion_tokens, 0);
    assert_eq!(response.usage.total_tokens, 0);
    let mut reply = decode_foundry_embedding(
        br#"{"object":"answer","model":"group","usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}"#,
    )
    .unwrap();
    assert!(reply.data.take().is_none());
    response.apply_reply(reply);
    assert_eq!(response.object, "answer");
    assert_eq!(response.model.as_deref(), Some("group"));
    assert_eq!(response.usage.prompt_tokens, 2);
    assert_eq!(response.usage.completion_tokens, 1);
    assert_eq!(response.usage.total_tokens, 3);
    assert!(response.additional_headers.is_empty());
}
