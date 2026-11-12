use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::embedding::EmbeddingError;
use betterlite_provider::operation::OperationError;
use betterlite_provider::transform_retry::TransformRetryError;
use http::{HeaderMap, HeaderValue};
use rstest::rstest;

use super::super::CallError;
use super::super::embedding::embedding_error;

fn provider_error() -> OperationError {
    OperationError::Provider {
        response: Box::new(UpstreamResponse {
            status: 429,
            headers: HeaderMap::new(),
            body: Vec::new(),
        }),
    }
}

fn bad_json() -> serde_json::Error {
    serde_json::from_str::<JsonValue>("{").unwrap_err()
}

#[rstest]
#[case::provider(provider_error(), |error| matches!(error, CallError::Route(EmbeddingError::Provider { response }) if response.status == 429))]
#[case::missing_key(OperationError::MissingApiKey, |error| matches!(error, CallError::Route(EmbeddingError::MissingApiKey)))]
#[case::timeout(OperationError::Timeout, |error| matches!(error, CallError::Route(EmbeddingError::Http { source: HttpError::Timeout })))]
#[case::too_large(OperationError::TooLarge { limit: 5 }, |error| matches!(error, CallError::Route(EmbeddingError::Http { source: HttpError::TooLarge { limit: 5 } })))]
#[case::encode(OperationError::Encode { source: bad_json() }, |error| matches!(error, CallError::Route(EmbeddingError::Encode { .. })))]
#[case::decode(OperationError::Decode { operation: "embedding", source: bad_json() }, |error| matches!(error, CallError::Route(EmbeddingError::Decode { .. })))]
#[case::unavailable_adapter(
    OperationError::UnavailableAdapter { provider: String::from("azure") },
    |error| matches!(error, CallError::Route(EmbeddingError::UnsupportedProvider { model }) if model == "azure")
)]
#[case::unsupported_operation(
    OperationError::UnsupportedOperation { provider: String::from("openai"), operation: String::from("rerank") },
    |error| matches!(error, CallError::Route(EmbeddingError::UnsupportedProvider { model }) if model == "openai")
)]
#[case::transport_failure_is_kept(
    OperationError::Http { source: TransportError::HeadersTooLarge { limit: 1 } },
    |error| matches!(error, CallError::Operation { source: OperationError::Http { source: TransportError::HeadersTooLarge { limit: 1 } } })
)]
#[case::invalid_header_is_kept(
    OperationError::InvalidHeader { name: String::from("x-a"), source: HeaderValue::from_str("\n").unwrap_err() },
    |error| matches!(error, CallError::Operation { source: OperationError::InvalidHeader { name, .. } } if name == "x-a")
)]
#[case::transform_retry_is_kept(
    OperationError::TransformRetry(TransformRetryError::Attempts { max_attempts: 0 }),
    |error| matches!(error, CallError::Operation { source: OperationError::TransformRetry(TransformRetryError::Attempts { max_attempts: 0 }) })
)]
#[test]
fn provider_framework_tests_behavior<Shape>(#[case] given: OperationError, #[case] shape: Shape)
where
    Shape: Fn(CallError<EmbeddingError>) -> bool,
{
    assert!(shape(embedding_error(given)));
}
