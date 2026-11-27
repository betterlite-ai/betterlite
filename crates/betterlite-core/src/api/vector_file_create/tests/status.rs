use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::Headers;
use http::HeaderMap as HttpHeaders;
use secrecy::SecretString;

use super::super::status::StatusCheckedFilesConfig;
use crate::api::vector_file_http_create_list::config::{FilesConfig, FilesParams};
use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap, UrlFailure};

struct Inner;

impl FilesConfig for Inner {
    fn validate_environment(&self, headers: Headers, _params: &FilesParams) -> Headers {
        let mut headers = headers;
        headers.insert(
            String::from("x-inner"),
            SecretString::from(String::from("yes")),
        );
        headers
    }

    fn complete_url(
        &self,
        _params: &FilesParams,
        vector_store_id: &str,
    ) -> Result<String, UrlFailure> {
        Ok(format!("https://inner.test/v1/{vector_store_id}"))
    }

    fn transform_response(&self, _reply: &UpstreamResponse) -> Result<Json, BaseLlmException> {
        Ok(Json::Text(String::from("inner answer")))
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: format!("inner: {message}"),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

fn reply(status: u16) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HttpHeaders::new(),
        body: br#"{"error":{"message":"bad"}}"#.to_vec(),
    }
}

fn params() -> FilesParams {
    FilesParams {
        api_base: None,
        api_key: None,
        ssl_verify: None,
    }
}

#[test]
fn status_check_rejects_error_replies() {
    let config = StatusCheckedFilesConfig { inner: Inner };
    let failure = config
        .transform_response(&reply(401))
        .expect_err("an error status fails before the provider transform");
    assert_eq!(failure.status_code, 401);
    assert_eq!(failure.message, r#"{"error":{"message":"bad"}}"#);
    let accepted = config
        .transform_response(&reply(200))
        .expect("a success status reaches the provider transform");
    assert_eq!(accepted, Json::Text(String::from("inner answer")));
    let boundary = config
        .transform_response(&reply(400))
        .expect_err("400 is an error status");
    assert_eq!(boundary.status_code, 400);
}

#[test]
fn status_check_delegates_provider_answers() {
    let config = StatusCheckedFilesConfig { inner: Inner };
    let headers = config.validate_environment(Headers::new(), &params());
    assert!(headers.contains_key("x-inner"));
    assert_eq!(
        config
            .complete_url(&params(), "vs_a")
            .expect("the inner url answers"),
        "https://inner.test/v1/vs_a"
    );
    let failure = config.error_class("bad", 418, HeaderMap::new());
    assert_eq!(failure.message, "inner: bad");
}
