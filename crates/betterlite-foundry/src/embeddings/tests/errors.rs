use secrecy::SecretString;

use crate::embeddings::call::FoundryEmbeddingRequest;
use crate::embeddings::error::FoundryEmbeddingError;
use crate::embeddings::reply::FoundryEmbeddingResponse;

use super::support::{
    Answer, BASE, Factory, IMAGE_ONE, IMAGE_URI, MODEL, TEXT_EMPTY, TEXT_ONE, Text, key, request,
    run, strings,
};

pub(super) fn embedding_error() {
    image_data_null();
    text_data_null();
    data_absent();
    transport_failure();
    answered_status();
    meaningful_error_in_reply();
    meaningful_error_fields();
    malformed_replies();
    conversion_failure();
    short_result_lists();
}

fn keyed<'call>(
    input: &'call [String],
    secret: &'call SecretString,
) -> FoundryEmbeddingRequest<'call> {
    let mut call = request(MODEL, input);
    call.api_key = Some(secret);
    call
}

// one run against a fresh image answer and text reply
fn answered(
    call: &FoundryEmbeddingRequest<'_>,
    image: Answer,
    text_body: &str,
) -> Result<FoundryEmbeddingResponse, FoundryEmbeddingError> {
    run(call, None, &Factory::new(image), &Text::replying(text_body))
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
    let no_data = Answer::json(r#"{"object":"list","model":"image-model"}"#);
    let image_error = answered(&call, no_data, TEXT_ONE).unwrap_err();
    assert!(matches!(image_error, FoundryEmbeddingError::DataMissing));
    assert_eq!(
        image_error.to_string(),
        "the provider answer has no data field"
    );
    let texts = strings(&["hello"]);
    let text_call = keyed(&texts, &secret);
    let text_error = answered(&text_call, Answer::json(IMAGE_ONE), "{}").unwrap_err();
    assert!(matches!(text_error, FoundryEmbeddingError::DataMissing));
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

fn meaningful_error_in_reply() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let call = keyed(&input, &secret);
    let entry = r#"[{"object":"embedding","index":0,"embedding":[0.1]}]"#;
    let quota = format!(r#"{{"error":{{"message":"quota","code":429}},"data":{entry}}}"#);
    assert!(matches!(
        answered(&call, Answer::json(&quota), TEXT_ONE).unwrap_err(),
        FoundryEmbeddingError::ReplyError { status: 429 }
    ));
    let boom = format!(r#"{{"error":"boom","data":{entry}}}"#);
    assert!(matches!(
        answered(&call, Answer::json(&boom), TEXT_ONE).unwrap_err(),
        FoundryEmbeddingError::ReplyError { status: 422 }
    ));
    let blank = format!(r#"{{"error":"","data":{entry}}}"#);
    let blank_answer = answered(&call, Answer::json(&blank), TEXT_ONE).unwrap();
    assert_eq!(blank_answer.data.len(), 1);
    let quiet = format!(r#"{{"error":{{"message":"","code":null}},"data":{entry}}}"#);
    let quiet_answer = answered(&call, Answer::json(&quiet), TEXT_ONE).unwrap();
    assert_eq!(quiet_answer.data.len(), 1);
    // the reply error is checked before the rest of the reply is decoded
    let shape = r#"{"error":{"message":"quota"},"data":"text"}"#;
    assert!(matches!(
        answered(&call, Answer::json(shape), TEXT_ONE).unwrap_err(),
        FoundryEmbeddingError::ReplyError { status: 422 }
    ));
}

fn meaningful_error_fields() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let call = keyed(&input, &secret);
    let entry = r#"[{"object":"embedding","index":0,"embedding":[0.1]}]"#;
    let message = format!(r#"{{"error":{{"message":"quota"}},"data":{entry}}}"#);
    assert!(matches!(
        answered(&call, Answer::json(&message), TEXT_ONE).unwrap_err(),
        FoundryEmbeddingError::ReplyError { status: 422 }
    ));
    let code = format!(r#"{{"error":{{"code":429}},"data":{entry}}}"#);
    assert!(matches!(
        answered(&call, Answer::json(&code), TEXT_ONE).unwrap_err(),
        FoundryEmbeddingError::ReplyError { status: 429 }
    ));
}

fn malformed_replies() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let call = keyed(&input, &secret);
    for body in ["not json", "[1,2]", r#"{"data":"text"}"#] {
        let error = answered(&call, Answer::json(body), TEXT_ONE).unwrap_err();
        assert!(
            matches!(error, FoundryEmbeddingError::Json { .. }),
            "{body}"
        );
    }
    let null_object = Answer::json(r#"{"object":null,"data":[]}"#);
    assert!(matches!(
        answered(&call, null_object, TEXT_ONE).unwrap_err(),
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
    let error = answered(&call, Answer::headed(&tokens, IMAGE_ONE), TEXT_ONE).unwrap_err();
    assert!(matches!(error, FoundryEmbeddingError::TokenCount { .. }));
    assert_eq!(
        error.to_string(),
        r#"the llm_provider-num_tokens header "abc" is not a token count"#
    );
    let negative = [("llm_provider-num_tokens", "-3")];
    assert!(matches!(
        answered(&call, Answer::headed(&negative, IMAGE_ONE), TEXT_ONE).unwrap_err(),
        FoundryEmbeddingError::TokenCount { .. }
    ));
}

fn short_result_lists() {
    let secret = key("case-key");
    let mixed = strings(&[IMAGE_URI, "hello"]);
    let mixed_call = keyed(&mixed, &secret);
    let empty_image = r#"{"object":"list","data":[],"model":"image-model"}"#;
    assert!(matches!(
        answered(&mixed_call, Answer::json(empty_image), TEXT_ONE).unwrap_err(),
        FoundryEmbeddingError::TooFewEntries {
            entries: "image embeddings"
        }
    ));
    let images = strings(&[IMAGE_URI, "hello", IMAGE_URI]);
    let images_call = keyed(&images, &secret);
    let two = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.1]},{"object":"embedding","index":1,"embedding":[0.2]}],"model":"image-model"}"#;
    assert!(matches!(
        answered(&images_call, Answer::json(two), TEXT_EMPTY).unwrap_err(),
        FoundryEmbeddingError::TooFewEntries {
            entries: "text embeddings"
        }
    ));
}
