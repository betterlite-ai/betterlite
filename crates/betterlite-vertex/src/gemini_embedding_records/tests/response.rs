use betterlite_openai::embedding::EmbeddingVector;
use rstest::rstest;

use super::support::CharacterCount;
use super::support::input;
use super::support::json;
use super::support::resolved_files;
use super::support::serialized;
use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::gemini_embedding_records::response::EmbeddingReply;
use crate::gemini_embedding_records::response::process_batch_reply;
use crate::gemini_embedding_records::response::process_embed_content_reply;

const MODEL: &str = "gemini-embedding-001";

fn batch_reply(source: &str, response: &str) -> Result<EmbeddingReply, GeminiEmbeddingError> {
    process_batch_reply(&input(source), MODEL, &json(response), &CharacterCount)
}

fn embed_reply(
    source: &str,
    response: &str,
    resolved: &str,
) -> Result<EmbeddingReply, GeminiEmbeddingError> {
    process_embed_content_reply(
        &input(source),
        MODEL,
        &json(response),
        &resolved_files(resolved),
        &CharacterCount,
    )
}

#[test]
fn batch_replies_index_in_embedding_order() {
    let reply = batch_reply(
        r#"["a","b"]"#,
        r#"{"embeddings":[{"values":[0.1,0.2]},{"values":[0.3]}]}"#,
    )
    .unwrap();
    assert_eq!(reply.model, MODEL);
    assert_eq!(reply.data.len(), 2);
    assert_eq!(reply.data[0].index, 0);
    assert_eq!(reply.data[1].index, 1);
    assert_eq!(reply.data[0].object, "embedding");
    assert_eq!(reply.usage.prompt_tokens, 2);
    assert_eq!(reply.usage.total_tokens, 2);
    assert_eq!(
        serialized(&reply),
        json(
            r#"{"model":"gemini-embedding-001","data":[
                {"object":"embedding","embedding":[0.1,0.2],"index":0},
                {"object":"embedding","embedding":[0.3],"index":1}
            ],"usage":{"prompt_tokens":2,"completion_tokens":0,"total_tokens":2,"prompt_tokens_details":null}}"#
        )
    );
}

#[test]
fn a_string_embedding_is_kept_as_base64() {
    let reply = batch_reply(r#""a""#, r#"{"embeddings":[{"values":"AAAA"}]}"#).unwrap();
    assert_eq!(
        reply.data[0].embedding,
        EmbeddingVector::Base64(String::from("AAAA"))
    );
}

#[rstest]
#[case::missing_embeddings("{}", "must hold an embeddings list")]
#[case::missing_values(r#"{"embeddings":[{}]}"#, "has no values")]
fn batch_replies_require_their_fields(#[case] response: &str, #[case] reason: &str) {
    let error = batch_reply(r#""a""#, response).unwrap_err();
    assert!(matches!(
        error,
        GeminiEmbeddingError::Reply { reason: found } if found.contains(reason)
    ));
}

#[test]
fn batch_replies_that_are_not_objects_are_rejected() {
    assert!(matches!(
        batch_reply(r#""a""#, "[]"),
        Err(GeminiEmbeddingError::Reply { .. })
    ));
}

#[test]
fn batch_embeddings_must_be_a_list() {
    assert!(matches!(
        batch_reply(r#""a""#, r#"{"embeddings":{}}"#),
        Err(GeminiEmbeddingError::Reply { .. })
    ));
}

#[test]
fn batch_embedding_entries_must_be_objects() {
    assert!(matches!(
        batch_reply(r#""a""#, r#"{"embeddings":[[]]}"#),
        Err(GeminiEmbeddingError::Reply { .. })
    ));
}

#[rstest]
#[case::boolean(r#"{"embeddings":[{"values":true}]}"#)]
#[case::number(r#"{"embeddings":[{"values":1}]}"#)]
#[case::object(r#"{"embeddings":[{"values":{}}]}"#)]
#[case::non_number_item(r#"{"embeddings":[{"values":[0.1,"0.2"]}]}"#)]
fn batch_embedding_values_must_be_numbers(#[case] response: &str) {
    assert!(matches!(
        batch_reply(r#""a""#, response),
        Err(GeminiEmbeddingError::Reply { .. })
    ));
}

#[test]
fn embed_content_replies_hold_one_indexed_record() {
    let reply = embed_reply(r#""hello""#, r#"{"embedding":{"values":[0.1]}}"#, "{}").unwrap();
    assert_eq!(reply.model, MODEL);
    assert_eq!(reply.data.len(), 1);
    assert_eq!(reply.data[0].index, 0);
    assert_eq!(reply.usage.prompt_tokens, 5);
    assert_eq!(reply.usage.total_tokens, 5);
    assert_eq!(reply.usage.prompt_tokens_details, None);
}

#[test]
fn embed_content_replies_require_the_embedding_field() {
    assert!(matches!(
        embed_reply(
            r#""a""#,
            r#"{"usageMetadata":{"promptTokenCount":2}}"#,
            "{}"
        ),
        Err(GeminiEmbeddingError::Reply {
            reason: "the embedContent reply has no embedding"
        })
    ));
}

#[test]
fn embed_content_replies_that_are_not_objects_are_rejected() {
    assert!(matches!(
        embed_reply(r#""a""#, "[]", "{}"),
        Err(GeminiEmbeddingError::Reply { .. })
    ));
}

#[test]
fn embed_content_usage_is_read_from_the_reply() {
    let reply = embed_reply(
        r#""data:image/png;base64,AA""#,
        r#"{"embedding":{"values":[0.1]},"usageMetadata":{"promptTokenCount":9,"totalTokenCount":9}}"#,
        "{}",
    )
    .unwrap();
    assert_eq!(
        serialized(&reply),
        json(
            r#"{"model":"gemini-embedding-001","data":[{"object":"embedding","embedding":[0.1],"index":0}],"usage":{"prompt_tokens":9,"completion_tokens":0,"total_tokens":9,"prompt_tokens_details":{"text_tokens":0,"audio_tokens":null,"image_tokens":9,"video_tokens":null}}}"#
        )
    );
}

#[test]
fn embed_content_usage_falls_back_on_an_invalid_record() {
    let reply = embed_reply(
        r#""hello""#,
        r#"{"embedding":{"values":[0.1]},"usageMetadata":{"promptTokenCount":"nine"}}"#,
        "{}",
    )
    .unwrap();
    assert_eq!(reply.usage.prompt_tokens, 5);
    assert_eq!(reply.usage.prompt_tokens_details, None);
}
