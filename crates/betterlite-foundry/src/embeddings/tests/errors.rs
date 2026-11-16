use secrecy::SecretString;

use crate::embeddings::call::FoundryEmbeddingRequest;
use crate::embeddings::error::FoundryEmbeddingError;

use super::support::{
    Answer, BASE, Factory, IMAGE_ONE, IMAGE_URI, MODEL, TEXT_ONE, Text, key, request, run, strings,
};

pub(super) fn embedding_error() {
    image_data_null();
    text_data_null();
    data_absent();
    transport_failure();
    answered_status();
    malformed_replies();
    conversion_failure();
}

fn keyed<'call>(
    input: &'call [String],
    secret: &'call SecretString,
) -> FoundryEmbeddingRequest<'call> {
    let mut call = request(MODEL, input);
    call.api_key = Some(secret);
    call
}

fn image_data_null() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let call = keyed(&input, &secret);
    let null_data = r#"{"object":"list","data":null,"model":"image-model"}"#;
    let factory = Factory::new(Answer::json(null_data));
    let text = Text::replying(TEXT_ONE);
    let error = run(&call, None, &factory, &text).unwrap_err();
    assert!(matches!(error, FoundryEmbeddingError::ImageDataMissing));
    assert_eq!(
        error.to_string(),
        "/image/embeddings route returned None Embeddings."
    );
    assert_eq!(factory.requests().len(), 1);
    assert!(text.calls().is_empty());
}

fn text_data_null() {
    let input = strings(&["hello"]);
    let secret = key("case-key");
    let call = keyed(&input, &secret);
    let null_data = r#"{"object":"list","data":null,"model":"text-model"}"#;
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    let error = run(&call, None, &factory, &Text::replying(null_data)).unwrap_err();
    assert!(matches!(error, FoundryEmbeddingError::TextDataMissing));
    assert_eq!(
        error.to_string(),
        "/v1/embeddings route returned None Embeddings."
    );
}

fn data_absent() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let call = keyed(&input, &secret);
    let factory = Factory::new(Answer::json(r#"{"object":"list","model":"image-model"}"#));
    let error = run(&call, None, &factory, &Text::replying(TEXT_ONE)).unwrap_err();
    assert!(matches!(error, FoundryEmbeddingError::DataMissing));
    assert_eq!(error.to_string(), "the provider answer has no data field");
    let texts = strings(&["hello"]);
    let call = keyed(&texts, &secret);
    let error = run(&call, None, &factory, &Text::replying(r#"{}"#)).unwrap_err();
    assert!(matches!(error, FoundryEmbeddingError::DataMissing));
}

fn transport_failure() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let call = keyed(&input, &secret);
    let factory = Factory::new(Answer::failed());
    let error = run(&call, None, &factory, &Text::replying(TEXT_ONE)).unwrap_err();
    assert!(matches!(error, FoundryEmbeddingError::Transport { .. }));
    assert_eq!(
        error.to_string(),
        format!("the request to {BASE}/models/images/embeddings failed")
    );
}

fn answered_status() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let call = keyed(&input, &secret);
    let factory = Factory::new(Answer::statused(503, IMAGE_ONE));
    let error = run(&call, None, &factory, &Text::replying(TEXT_ONE)).unwrap_err();
    assert!(matches!(
        error,
        FoundryEmbeddingError::Provider { status: 503 }
    ));
    assert_eq!(error.to_string(), "the provider answered with status 503");
}

fn malformed_replies() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let call = keyed(&input, &secret);
    for body in ["not json", "[1,2]", r#"{"data":"text"}"#] {
        let factory = Factory::new(Answer::json(body));
        let error = run(&call, None, &factory, &Text::replying(TEXT_ONE)).unwrap_err();
        assert!(matches!(error, FoundryEmbeddingError::Json { .. }), "{body}");
    }
    let factory = Factory::new(Answer::json(r#"{"object":null,"data":[]}"#));
    let error = run(&call, None, &factory, &Text::replying(TEXT_ONE)).unwrap_err();
    assert!(matches!(
        error,
        FoundryEmbeddingError::FieldType {
            field: "object",
            ..
        }
    ));
}

fn conversion_failure() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let call = keyed(&input, &secret);
    let tokens = [("llm_provider-num_tokens", "abc")];
    let factory = Factory::new(Answer::headed(&tokens, IMAGE_ONE));
    let error = run(&call, None, &factory, &Text::replying(TEXT_ONE)).unwrap_err();
    assert!(matches!(error, FoundryEmbeddingError::TokenCount { .. }));
    assert_eq!(
        error.to_string(),
        r#"the llm_provider-num_tokens header "abc" is not a token count"#
    );
    let negative = [("llm_provider-num_tokens", "-3")];
    let factory = Factory::new(Answer::headed(&negative, IMAGE_ONE));
    let error = run(&call, None, &factory, &Text::replying(TEXT_ONE)).unwrap_err();
    assert!(matches!(error, FoundryEmbeddingError::TokenCount { .. }));
}
