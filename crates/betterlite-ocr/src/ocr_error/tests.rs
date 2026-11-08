#![cfg(test)]

use std::io;
use std::path::PathBuf;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderValue;
use rstest::rstest;
use tokio::task::JoinSet;

use super::{
    OcrAuthError, OcrError, OcrHeaderError, OcrParamsError, OcrSecretError, OcrTransportError,
};

#[rstest]
#[case::empty_file(OcrError::EmptyFile, 400, "invalid_request_error", true, false)]
#[case::invalid_mime_type(OcrError::InvalidMimeType("image/x".to_owned()), 400, "invalid_request_error", true, false)]
#[case::cohere_image_only(OcrError::CohereImageOnly, 400, "invalid_request_error", true, false)]
#[case::request_format(OcrError::RequestFormat, 400, "invalid_request_error", true, false)]
#[case::request_field(OcrError::RequestField { path: "document".to_owned() }, 400, "invalid_request_error", true, false)]
#[case::missing_field(
    OcrError::MissingField("model"),
    400,
    "invalid_request_error",
    true,
    false
)]
#[case::missing_document_url(
    OcrError::MissingDocumentUrl,
    400,
    "invalid_request_error",
    true,
    false
)]
#[case::invalid_data_uri(OcrError::InvalidDataUri, 400, "invalid_request_error", true, false)]
#[case::reducto_source(OcrError::ReductoSource, 400, "invalid_request_error", true, false)]
#[case::inline_too_large(
    OcrError::InlineDocumentTooLarge,
    400,
    "invalid_request_error",
    true,
    false
)]
#[case::blocked_document_url(
    OcrError::BlockedDocumentUrl,
    400,
    "invalid_request_error",
    true,
    false
)]
#[case::download_disabled(OcrError::DownloadDisabled, 400, "invalid_request_error", true, false)]
#[case::download_too_large(OcrError::DownloadTooLarge, 400, "invalid_request_error", true, false)]
#[case::too_many_redirects(OcrError::TooManyRedirects, 400, "invalid_request_error", true, false)]
#[case::pages(OcrError::Pages("1-".to_owned()), 400, "invalid_request_error", true, false)]
#[case::features(OcrError::Features, 400, "invalid_request_error", true, false)]
#[case::dot_model(OcrError::DotModel, 400, "invalid_request_error", true, false)]
#[case::invalid_request(OcrError::InvalidRequest("empty".to_owned()), 400, "invalid_request_error", true, false)]
#[case::invalid_provider(OcrError::InvalidProvider("openai".to_owned()), 400, "invalid_request_error", true, false)]
#[case::invalid_model(OcrError::InvalidModel { provider: "mistral", model: "x".to_owned(), supported: &["a"] }, 400, "invalid_request_error", true, false)]
#[case::params(
    OcrError::Params(OcrParamsError::Body),
    400,
    "invalid_request_error",
    true,
    false
)]
#[case::headers(OcrError::Headers(OcrHeaderError::Value { name: "x".to_owned(), source: HeaderValue::from_str("a\nb").expect_err("invalid header value") }), 400, "invalid_request_error", true, false)]
#[case::http(
    OcrError::Http(HttpError::Timeout),
    400,
    "invalid_request_error",
    true,
    false
)]
#[case::too_large(OcrError::TooLarge { limit: 7 }, 500, "internal_server_error", false, true)]
#[case::response_field(OcrError::ResponseField { path: "pages".to_owned() }, 500, "internal_server_error", false, true)]
#[case::empty_content(OcrError::EmptyContent, 500, "internal_server_error", false, true)]
#[case::missing_redirect_location(
    OcrError::MissingRedirectLocation,
    500,
    "internal_server_error",
    false,
    true
)]
#[case::invalid_redirect(OcrError::InvalidRedirect, 500, "internal_server_error", false, true)]
#[case::operation_status(OcrError::OperationStatus("failed".to_owned()), 500, "internal_server_error", false, true)]
#[case::numeric_range(
    OcrError::NumericRange("dpi"),
    500,
    "internal_server_error",
    false,
    true
)]
#[case::poll_location(OcrError::PollLocation, 500, "internal_server_error", false, true)]
#[case::poll_origin(OcrError::PollOrigin, 500, "internal_server_error", false, true)]
#[case::poll_timeout(OcrError::PollTimeout, 500, "internal_server_error", false, true)]
#[case::invalid_response(OcrError::InvalidResponse("bad".to_owned()), 500, "internal_server_error", false, true)]
#[case::file_read(OcrError::FileRead { path: PathBuf::from("/tmp/x"), source: io::Error::other("disk") }, 500, "internal_server_error", false, false)]
#[case::unsupported(
    OcrError::Unsupported("streaming"),
    500,
    "internal_server_error",
    false,
    false
)]
#[case::missing_azure_ai(
    OcrError::MissingAzureAiCredentials,
    500,
    "internal_server_error",
    false,
    false
)]
#[case::missing_doc_intel(
    OcrError::MissingDocIntelCredentials,
    500,
    "internal_server_error",
    false,
    false
)]
#[case::missing_reducto(
    OcrError::MissingReductoApiKey,
    500,
    "internal_server_error",
    false,
    false
)]
#[case::secret(OcrError::Secret(OcrSecretError::Resolution { name: "K".to_owned(), detail: "x".to_owned() }), 500, "internal_server_error", false, false)]
#[case::auth(OcrError::Auth(OcrAuthError::MissingApiKey { provider: "Mistral", environment_variable: "MISTRAL_API_KEY" }), 500, "internal_server_error", false, false)]
#[case::transport_failed(
    OcrError::Transport(OcrTransportError::Failed(TransportError::Closed)),
    500,
    "internal_server_error",
    false,
    false
)]
#[case::provider_429(OcrError::Provider { status: 429, body: "slow down".to_owned(), headers: Vec::new() }, 429, "rate_limit_error", false, false)]
#[case::provider_401(OcrError::Provider { status: 401, body: "no key".to_owned(), headers: Vec::new() }, 401, "authentication_error", false, false)]
#[case::provider_403(OcrError::Provider { status: 403, body: "no".to_owned(), headers: Vec::new() }, 403, "permission_error", false, false)]
#[case::provider_404(OcrError::Provider { status: 404, body: "gone".to_owned(), headers: Vec::new() }, 404, "invalid_request_error", false, false)]
#[case::transport_503(OcrError::Transport(OcrTransportError::Http { status: 503, body: "later".to_owned() }), 503, "internal_server_error", false, false)]
#[case::transport_timeout(
    OcrError::Transport(OcrTransportError::Timeout),
    408,
    "invalid_request_error",
    false,
    false
)]
fn every_variant_carries_litellm_status_type_param_and_code(
    #[case] error: OcrError,
    #[case] status: u16,
    #[case] error_type: &str,
    #[case] request: bool,
    #[case] response: bool,
) {
    assert_eq!(error.http_status_code().unwrap_or(500), status);
    assert_eq!(error.is_request(), request);
    assert_eq!(error.is_response(), response);
    let fields = error.fields();
    assert_eq!(fields.status, status);
    assert_eq!(fields.error_type, error_type);
    assert_eq!(fields.param, None);
    assert_eq!(fields.code, status.to_string());
}

#[tokio::test]
async fn document_task_failures_are_statusless_server_errors() {
    let mut tasks = JoinSet::new();
    tasks.spawn(async { panic!("the preparation fails") });
    let join_error = tasks
        .join_next()
        .await
        .expect("the task runs")
        .expect_err("the task failed");
    let error = OcrError::DocumentTask(join_error);
    assert_eq!(error.http_status_code(), None);
    assert_eq!(error.fields().status, 500);
    assert_eq!(error.fields().error_type, "internal_server_error");
    assert!(!error.is_request());
    assert!(!error.is_response());
}
