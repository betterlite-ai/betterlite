use betterlite_openai::cohere_embedding::CohereEmbeddingError;
use betterlite_openai::infinity_embedding::error::InfinityEmbeddingError;
use rstest::rstest;

use super::*;
#[test]
fn cohere_embedding_uses_the_caller_headers_with_the_extras() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Cohere, "embed-v4.0");
    fixture.setup.api_key = Some(key("explicit"));
    fixture.setup.headers = Some(headers(&[
        ("authorization", "Bearer caller"),
        ("x-call", "1"),
    ]));
    fixture.setup.extra = headers(&[("x-extra", "2")]);
    let prepared = fixture.prepared();
    assert_eq!(
        header_of(&prepared, "authorization").as_deref(),
        Some("Bearer caller")
    );
    assert_eq!(header_of(&prepared, "x-call").as_deref(), Some("1"));
    assert_eq!(header_of(&prepared, "x-extra").as_deref(), Some("2"));
    assert_eq!(
        header_of(&prepared, "content-type").as_deref(),
        Some("application/json")
    );
    let body = body_of(&prepared);
    assert_eq!(text_field(&body, "model").as_deref(), Some("embed-v4.0"));
    assert_eq!(items_field(&body, "texts")[0].as_str(), Some("hi"));
}

#[test]
fn cohere_embedding_reads_the_key_chain() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Cohere, "embed-v4.0");
    fixture.setup.api_key = None;
    fixture.setup.settings.cohere_key = Some(key("first"));
    assert_eq!(
        header_of(&fixture.prepared(), "authorization").as_deref(),
        Some("Bearer first")
    );
    fixture.setup.settings.cohere_key = None;
    fixture.setup.settings.cohere_api_key = Some(key("second"));
    assert_eq!(
        header_of(&fixture.prepared(), "authorization").as_deref(),
        Some("Bearer second")
    );
    fixture.setup.settings.cohere_api_key = None;
    fixture.setup.settings.co_api_key = Some(key("third"));
    assert_eq!(
        header_of(&fixture.prepared(), "authorization").as_deref(),
        Some("Bearer third")
    );
    fixture.setup.settings.co_api_key = None;
    fixture.setup.settings.api_key = Some(key("last"));
    assert_eq!(
        header_of(&fixture.prepared(), "authorization").as_deref(),
        Some("Bearer last")
    );
}

#[test]
fn cohere_embedding_ignores_the_chat_base_globals() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Cohere, "embed-v4.0");
    fixture.setup.settings.api_base = Some(String::from("https://litellm"));
    fixture.setup.settings.cohere_api_base = Some(String::from("https://cohere"));
    assert_eq!(
        url_of(&fixture.prepared()),
        "https://api.cohere.ai/v2/embed"
    );
    fixture.setup.api_base = Some(String::from("https://explicit/embed"));
    assert_eq!(url_of(&fixture.prepared()), "https://explicit/embed");
}

#[test]
fn cohere_embedding_maps_the_openai_parameters() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Cohere, "embed-v4.0");
    fixture.request = serde_json::from_str(
        r#"{"input":"hi","dimensions":512,"encoding_format":"float","user":"someone"}"#,
    )
    .unwrap();
    let body = body_of(&fixture.prepared());
    assert_eq!(number_field(&body, "output_dimension"), Some(512));
    assert_eq!(
        items_field(&body, "embedding_types")[0].as_str(),
        Some("float")
    );
    assert!(body.get("user").is_none());
    assert_eq!(
        text_field(&body, "input_type").as_deref(),
        Some("search_document")
    );
}

#[test]
fn cohere_embedding_refuses_input_it_cannot_send_as_text() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Cohere, "embed-v4.0");
    fixture.request = serde_json::from_str(r#"{"input":[1,2]}"#).unwrap();
    assert!(matches!(
        fixture.prepare(),
        Err(FamilyError::CohereEmbedding(
            CohereEmbeddingError::InputNotText
        ))
    ));
    fixture.request = serde_json::from_str(r#"{"input":[]}"#).unwrap();
    assert!(matches!(
        fixture.prepare(),
        Err(FamilyError::CohereEmbedding(
            CohereEmbeddingError::EmptyInput
        ))
    ));
}

#[test]
fn voyage_embedding_sends_only_its_authorization() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Voyage, "voyage-3");
    fixture.setup.api_key = Some(key("voyage-key"));
    fixture.setup.headers = Some(headers(&[("x-call", "1")]));
    fixture.setup.extra = headers(&[("x-extra", "2")]);
    let prepared = fixture.prepared();
    assert_eq!(
        header_of(&prepared, "authorization").as_deref(),
        Some("Bearer voyage-key")
    );
    assert_eq!(header_of(&prepared, "accept"), None);
    assert_eq!(header_of(&prepared, "x-call"), None);
    assert_eq!(header_of(&prepared, "x-extra"), None);
    assert_eq!(url_of(&prepared), "https://api.voyageai.com/v1/embeddings");
    assert_eq!(
        text_field(&body_of(&prepared), "input").as_deref(),
        Some("hi")
    );
}

#[test]
fn voyage_embedding_reads_the_key_from_the_environment() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Voyage, "voyage-3");
    fixture.setup.api_key = None;
    fixture
        .setup
        .environment
        .insert(String::from("VOYAGE_API_KEY"), String::from("env-key"));
    assert_eq!(
        header_of(&fixture.prepared(), "authorization").as_deref(),
        Some("Bearer env-key")
    );
    fixture.setup.environment.clear();
    assert_eq!(
        header_of(&fixture.prepared(), "authorization").as_deref(),
        Some("Bearer None")
    );
}

#[test]
fn voyage_variants_follow_the_model_text() {
    let contextual = EmbeddingFixture::new(ProviderId::Voyage, "voyage-context-3").prepared();
    assert_eq!(
        url_of(&contextual),
        "https://api.voyageai.com/v1/contextualizedembeddings"
    );
    let body = body_of(&contextual);
    assert_eq!(items_field(&body, "inputs")[0].as_str(), Some("hi"));
    assert_eq!(flag_field(&body, "enable_auto_chunking"), Some(true));
    assert_eq!(number_field(&body, "chunk_size"), Some(32_000));
    assert_eq!(text_field(&body, "input_type").as_deref(), Some("document"));

    let multimodal = EmbeddingFixture::new(ProviderId::Voyage, "voyage-multimodal-3").prepared();
    assert_eq!(
        url_of(&multimodal),
        "https://api.voyageai.com/v1/multimodalembeddings"
    );
    // the leaf sends the item as given, so LiteLLM's content wrapper is a listed deviation
    let multimodal_body = body_of(&multimodal);
    assert_eq!(
        text_field(&multimodal_body, "inputs").as_deref(),
        Some("hi")
    );
    assert!(multimodal_body.get("enable_auto_chunking").is_none());

    let text = EmbeddingFixture::new(ProviderId::Voyage, "voyage-3").prepared();
    assert!(body_of(&text).get("inputs").is_none());
}

#[test]
fn voyage_embedding_keeps_only_the_supported_options() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Voyage, "voyage-3");
    fixture.request = serde_json::from_str(
        r#"{"input":"hi","dimensions":256,"encoding_format":"float","user":"someone"}"#,
    )
    .unwrap();
    let body = body_of(&fixture.prepared());
    assert_eq!(number_field(&body, "output_dimension"), Some(256));
    assert_eq!(
        text_field(&body, "encoding_format").as_deref(),
        Some("float")
    );
    assert!(body.get("user").is_none());
    assert!(body.get("dimensions").is_none());
}

#[test]
fn jina_embedding_wraps_a_scalar_and_maps_dimensions() {
    let mut fixture = EmbeddingFixture::new(ProviderId::JinaAi, "jina-embeddings-v3");
    fixture.setup.api_base = Some(String::from("https://jina.test/v1"));
    fixture.request = serde_json::from_str(r#"{"input":"hi","dimensions":128}"#).unwrap();
    let prepared = fixture.prepared();
    assert_eq!(url_of(&prepared), "https://jina.test/v1/embeddings");
    let body = body_of(&prepared);
    assert_eq!(items_field(&body, "input")[0].as_str(), Some("hi"));
    assert_eq!(number_field(&body, "dimensions"), Some(128));
    assert_eq!(
        header_of(&prepared, "authorization").as_deref(),
        Some("Bearer key")
    );
}

#[test]
fn jina_embedding_resolves_base_and_key_from_the_host_settings() {
    let mut fixture = EmbeddingFixture::new(ProviderId::JinaAi, "jina-embeddings-v3");
    fixture.setup.api_key = None;
    fixture.setup.host.jina_ai_api_base = Some(String::from("https://host.test"));
    fixture.setup.host.jina_ai_api_key = Some(key("host-key"));
    let prepared = fixture.prepared();
    assert_eq!(url_of(&prepared), "https://host.test/embeddings");
    assert_eq!(
        header_of(&prepared, "authorization").as_deref(),
        Some("Bearer host-key")
    );
}

#[test]
fn infinity_embedding_requires_a_base() {
    let fixture = EmbeddingFixture::new(ProviderId::Infinity, "infinity-model");
    assert!(matches!(
        fixture.prepare(),
        Err(FamilyError::InfinityEmbedding(
            InfinityEmbeddingError::MissingApiBase
        ))
    ));
}

#[test]
fn infinity_embedding_sends_bearer_none_without_a_key() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Infinity, "infinity-model");
    fixture.setup.api_key = None;
    fixture.setup.api_base = Some(String::from("https://infinity.test"));
    let prepared = fixture.prepared();
    assert_eq!(url_of(&prepared), "https://infinity.test/embeddings");
    assert_eq!(
        header_of(&prepared, "authorization").as_deref(),
        Some("Bearer None")
    );
    assert_eq!(
        text_field(&body_of(&prepared), "model").as_deref(),
        Some("infinity-model")
    );
}

// a null passthrough option is dropped before a family's mapper can copy it into the body
#[test]
fn a_null_passthrough_option_never_reaches_an_embedding_body() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Infinity, "infinity-model");
    fixture.setup.api_base = Some(String::from("https://infinity.test"));
    fixture.request = serde_json::from_str(r#"{"input":"hi","modality":null}"#).unwrap();
    assert!(body_of(&fixture.prepared()).get("modality").is_none());
}

#[test]
fn an_unsupported_provider_prepares_no_embedding() {
    let fixture = EmbeddingFixture::new(ProviderId::Openai, "text-embedding-3-small");
    assert!(matches!(
        fixture.prepare(),
        Err(FamilyError::Unsupported { .. })
    ));
}
#[test]
fn a_cohere_embedding_reply_is_decoded() {
    let prepared = EmbeddingFixture::new(ProviderId::Cohere, "embed-v4.0").prepared();
    let answer = embedding_result(decode(&prepared, 200, COHERE_EMBEDDING_REPLY).unwrap());
    let EmbeddingRerankResult::CohereEmbedding(reply) = answer else {
        panic!("the cohere embedding reply");
    };
    assert_eq!(reply.object, "list");
    assert_eq!(reply.data.len(), 1);
    assert_eq!(reply.usage.prompt_tokens, 2);
}

#[test]
fn a_voyage_embedding_reply_is_projected() {
    let prepared = EmbeddingFixture::new(ProviderId::Voyage, "voyage-3").prepared();
    let answer = embedding_result(decode(&prepared, 200, VOYAGE_EMBEDDING_REPLY).unwrap());
    let EmbeddingRerankResult::VoyageEmbedding(reply) = answer else {
        panic!("the voyage embedding reply");
    };
    let body = reply.as_object().unwrap();
    assert_eq!(text_field(body, "model").as_deref(), Some("voyage-3"));
    let usage = object_field(body, "usage").unwrap();
    assert_eq!(number_field(usage, "total_tokens"), Some(3));
    assert_eq!(number_field(usage, "prompt_tokens"), Some(3));
}

#[test]
fn a_jina_embedding_reply_is_decoded() {
    let prepared = EmbeddingFixture::new(ProviderId::JinaAi, "jina-embeddings-v3").prepared();
    let answer = embedding_result(decode(&prepared, 200, JINA_EMBEDDING_REPLY).unwrap());
    let EmbeddingRerankResult::JinaEmbedding(reply) = answer else {
        panic!("the jina embedding reply");
    };
    assert_eq!(reply.data.len(), 1);
    assert_eq!(reply.usage.prompt_tokens, 2);
}

#[test]
fn an_infinity_embedding_reply_is_decoded() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Infinity, "infinity-model");
    fixture.setup.api_base = Some(String::from("https://infinity.test"));
    let prepared = fixture.prepared();
    let answer = embedding_result(decode(&prepared, 200, INFINITY_EMBEDDING_REPLY).unwrap());
    let EmbeddingRerankResult::InfinityEmbedding(reply) = answer else {
        panic!("the infinity embedding reply");
    };
    assert_eq!(reply.model.as_deref(), Some("infinity-model"));
    assert_eq!(reply.usage.prompt_tokens, 2);
}

#[test]
fn cohere_embedding_counts_the_input_when_usage_is_missing() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Cohere, "embed-v4.0");
    fixture.request =
        serde_json::from_str(r#"{"input":"the quick brown fox jumps over the lazy dog"}"#).unwrap();
    let prepared = fixture.prepared();
    let reply = r#"{"embeddings":{"float":[[0.1,0.2]]},"meta":{"billed_units":{}}}"#;
    let answer = embedding_result(decode(&prepared, 200, reply).unwrap());
    let EmbeddingRerankResult::CohereEmbedding(decoded) = answer else {
        panic!("the cohere embedding reply");
    };
    // the logger carries the input the tokenizer counts, and the sentence is nine cl100k tokens
    assert_eq!(decoded.usage.prompt_tokens, 9);
    assert_eq!(decoded.usage.total_tokens, 9);
    assert!(decoded.usage.prompt_tokens_details.is_none());
}

#[test]
fn cohere_embedding_sends_a_batch_of_texts() {
    let mut fixture = EmbeddingFixture::new(ProviderId::Cohere, "embed-v4.0");
    fixture.request = serde_json::from_str(r#"{"input":["one","two"]}"#).unwrap();
    let body = body_of(&fixture.prepared());
    let texts = items_field(&body, "texts");
    assert_eq!(texts.len(), 2);
    assert_eq!(texts[0].as_str(), Some("one"));
    assert_eq!(texts[1].as_str(), Some("two"));
    assert_eq!(
        text_field(&body, "input_type").as_deref(),
        Some("search_document")
    );
}

#[rstest]
#[case(ProviderId::Cohere, "embed-v4.0", "cohere")]
#[case(ProviderId::Voyage, "voyage-3", "voyage")]
#[case(ProviderId::JinaAi, "jina-embeddings-v3", "jina")]
#[case(ProviderId::Infinity, "infinity-model", "infinity")]
fn an_unparsable_embedding_base_names_each_family(
    #[case] provider: ProviderId,
    #[case] model: &str,
    #[case] expected: &str,
) {
    let mut fixture = EmbeddingFixture::new(provider, model);
    fixture.setup.api_base = Some(String::from("not a url"));
    assert!(
        matches!(fixture.prepare(), Err(FamilyError::Url { family, .. }) if family == expected),
        "the {expected} URL failure"
    );
}
