#![cfg(test)]

use std::any::type_name;
use std::collections::BTreeMap;

use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::hosted_vllm::chat::ChatConfig;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest,
};
use rstest::rstest;
use secrecy::SecretString;

use super::VllmConfig;
use super::completion::ENGINE_ERROR;
use super::error::{
    ENGINE_MISSING_MESSAGE, ENGINE_STATUS, VllmError, VllmSetupError, get_error_class,
};
use super::model_info::{API_BASE_ENV, API_KEY_HEADER, VllmModelInfo};
use super::passthrough::VllmPassthroughConfig;

fn environment(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn info(environment: &BTreeMap<String, String>) -> VllmModelInfo<'_> {
    VllmModelInfo { environment }
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn query(wire: &str) -> Object {
    serde_json::from_str(wire).unwrap()
}

fn url_request<'call>(
    api_base: Option<&'call str>,
    endpoint: &'call str,
    request_query_params: Option<&'call Object>,
    litellm_params: &'call Object,
) -> PassthroughUrlRequest<'call> {
    PassthroughUrlRequest {
        api_base,
        api_key: None,
        target: PassthroughTarget {
            model: "m",
            custom_llm_provider: "vllm",
            endpoint,
        },
        request_query_params,
        litellm_params,
    }
}

#[test]
fn vllm_config_is_the_hosted_vllm_chat_config() {
    assert_eq!(type_name::<VllmConfig>(), type_name::<ChatConfig>());
}

#[test]
fn validate_environment_adds_the_supplied_key() {
    let environment = environment(&[]);
    let mut headers = Headers::from([(String::from("X"), String::from("1"))]);
    info(&environment).validate_environment(&mut headers, Some(&secret("sk-x")));
    assert_eq!(
        headers,
        Headers::from([
            (String::from(API_KEY_HEADER), String::from("sk-x")),
            (String::from("X"), String::from("1")),
        ])
    );
}

#[test]
fn validate_environment_without_a_key_keeps_the_given_headers() {
    let environment = environment(&[]);
    let mut headers = Headers::from([(String::from(API_KEY_HEADER), String::from("given"))]);
    info(&environment).validate_environment(&mut headers, None);
    assert_eq!(
        headers,
        Headers::from([(String::from(API_KEY_HEADER), String::from("given"))])
    );
}

#[test]
fn validate_environment_sends_an_empty_key_as_given() {
    let environment = environment(&[]);
    let mut headers = Headers::new();
    info(&environment).validate_environment(&mut headers, Some(&secret("")));
    assert_eq!(headers.get(API_KEY_HEADER).map(String::as_str), Some(""));
}

#[rstest]
#[case::supplied(Some("http://supplied.test"), None, Some("http://supplied.test"))]
#[case::from_environment(None, Some("http://env.test"), Some("http://env.test"))]
#[case::supplied_wins(
    Some("http://supplied.test"),
    Some("http://env.test"),
    Some("http://supplied.test")
)]
#[case::empty_supplied_falls_through(Some(""), Some("http://env.test"), Some("http://env.test"))]
#[case::empty_environment_is_unset(
    Some("http://supplied.test"),
    Some(""),
    Some("http://supplied.test")
)]
#[case::supplied_whitespace_is_given(Some(" "), None, Some(" "))]
#[case::absent(None, None, None)]
#[case::empty_supplied_and_environment(Some(""), Some(""), None)]
fn the_api_base_is_the_supplied_one_then_the_environment(
    #[case] api_base: Option<&str>,
    #[case] env_base: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let values: Vec<(&str, &str)> = env_base
        .map(|base| (API_BASE_ENV, base))
        .into_iter()
        .collect();
    let environment = environment(&values);
    let resolved = info(&environment).get_api_base(api_base);
    assert_eq!(resolved.ok().as_deref(), expected);
}

#[rstest]
#[case::absent(None, None)]
#[case::empty_supplied(Some(""), Some(""))]
#[case::empty_environment(None, Some(""))]
fn a_missing_base_reports_litellms_pass_through_text(
    #[case] api_base: Option<&str>,
    #[case] env_base: Option<&str>,
) {
    let values: Vec<(&str, &str)> = env_base
        .map(|base| (API_BASE_ENV, base))
        .into_iter()
        .collect();
    let environment = environment(&values);
    let error = info(&environment).get_api_base(api_base).unwrap_err();
    assert_eq!(error, VllmSetupError::MissingApiBase);
    assert_eq!(
        error.to_string(),
        "VLLM_API_BASE is not set. Please set the environment variable, to use VLLM's pass-through - `{LITELLM_API_BASE}/vllm/{endpoint}`."
    );
}

#[test]
fn get_models_reports_the_missing_credentials_after_a_base_resolves() {
    let environment = environment(&[]);
    let error = info(&environment)
        .get_models(Some("http://vllm.test"))
        .unwrap_err();
    assert_eq!(error, VllmSetupError::MissingModelsCredentials);
    assert_eq!(
        error.to_string(),
        "VLLM_API_BASE or VLLM_API_KEY is not set. Please set the environment variable, to query VLLM's `/models` endpoint."
    );
}

#[test]
fn get_models_never_reads_a_key_so_it_always_fails() {
    let environment = environment(&[(API_BASE_ENV, "http://env.test")]);
    let error = info(&environment).get_models(None).unwrap_err();
    assert_eq!(error, VllmSetupError::MissingModelsCredentials);
}

#[test]
fn get_models_without_a_base_reports_the_missing_base_first() {
    let environment = environment(&[]);
    let error = info(&environment).get_models(None).unwrap_err();
    assert_eq!(error, VllmSetupError::MissingApiBase);
}

#[rstest]
#[case::absent("{}", false)]
#[case::false_value(r#"{"stream":false}"#, true)]
#[case::null_value(r#"{"stream":null}"#, true)]
#[case::zero(r#"{"stream":0}"#, true)]
#[case::other_keys(r#"{"temperature":0.2}"#, false)]
fn streaming_is_the_presence_of_the_stream_key(#[case] wire: &str, #[case] streams: bool) {
    let environment = environment(&[]);
    let config = VllmPassthroughConfig {
        environment: &environment,
    };
    assert_eq!(
        config.is_streaming_request("/v1/chat/completions", &query(wire)),
        streams
    );
}

#[rstest]
#[case::plain(
    None,
    Some("http://vllm.test"),
    "/v1/models",
    "{}",
    "http://vllm.test/v1/models"
)]
#[case::trailing_slash(
    None,
    Some("http://vllm.test/"),
    "v1/models",
    "{}",
    "http://vllm.test/v1/models"
)]
#[case::leading_slash_on_base(
    None,
    Some("http://vllm.test//"),
    "//v1/models",
    "{}",
    "http://vllm.test/v1/models"
)]
#[case::environment_base(
    None,
    Some("http://env.test"),
    "/v1/models",
    "{}",
    "http://env.test/v1/models"
)]
#[case::query(
    Some("http://vllm.test"),
    None,
    "/v1/models",
    r#"{"a":"b"}"#,
    "http://vllm.test/v1/models?a=b"
)]
#[case::number_query(
    None,
    Some("http://vllm.test"),
    "/v1/models",
    r#"{"limit":3}"#,
    "http://vllm.test/v1/models?limit=3"
)]
#[case::space_query(
    None,
    Some("http://vllm.test"),
    "/v1/models",
    r#"{"a":"b c"}"#,
    "http://vllm.test/v1/models?a=b+c"
)]
#[case::empty_query(
    None,
    Some("http://vllm.test"),
    "/v1/models",
    "{}",
    "http://vllm.test/v1/models"
)]
#[case::existing_query(
    None,
    Some("http://vllm.test"),
    "/v1/models?x=1",
    r#"{"a":"b"}"#,
    "http://vllm.test/v1/models?a=b"
)]
fn the_complete_url_joins_the_base_and_the_endpoint(
    #[case] supplied: Option<&str>,
    #[case] env_base: Option<&str>,
    #[case] endpoint: &str,
    #[case] query_wire: &str,
    #[case] expected: &str,
) {
    let values: Vec<(&str, &str)> = env_base
        .map(|base| (API_BASE_ENV, base))
        .into_iter()
        .collect();
    let environment = environment(&values);
    let config = VllmPassthroughConfig {
        environment: &environment,
    };
    let params = query(query_wire);
    let empty = Object::new();
    let request = url_request(supplied, endpoint, Some(&params), &empty);
    let (url, base) = config.get_complete_url(&request).unwrap();
    assert_eq!(url.as_str(), expected);
    assert_eq!(Some(base.as_str()), supplied.or(env_base));
}

#[test]
fn the_complete_url_reports_the_missing_base() {
    let environment = environment(&[]);
    let config = VllmPassthroughConfig {
        environment: &environment,
    };
    let empty = Object::new();
    let request = url_request(None, "/v1/models", None, &empty);
    let error = config.get_complete_url(&request).unwrap_err();
    assert!(matches!(
        error,
        VllmError::Setup(VllmSetupError::MissingApiBase)
    ));
    assert_eq!(error.status(), None);
}

#[test]
fn an_unparsable_base_is_a_passthrough_error() {
    let environment = environment(&[]);
    let config = VllmPassthroughConfig {
        environment: &environment,
    };
    let empty = Object::new();
    let request = url_request(Some("not a url"), "/v1/models", None, &empty);
    let error = config.get_complete_url(&request).unwrap_err();
    assert!(matches!(error, VllmError::Passthrough(_)));
    assert_eq!(error.status(), None);
}

#[test]
fn the_engine_path_answers_not_supported() {
    assert_eq!(ENGINE_ERROR.status(), Some(ENGINE_STATUS));
    assert_eq!(ENGINE_STATUS, 400);
    assert_eq!(ENGINE_ERROR.to_string(), ENGINE_MISSING_MESSAGE);
    assert_eq!(ENGINE_MISSING_MESSAGE, "No module named 'vllm'");
}

#[test]
fn an_api_error_keeps_the_status_message_and_headers() {
    let environment = environment(&[]);
    let headers = Headers::from([(String::from("x"), String::from("1"))]);
    let error = info(&environment).get_error_class("boom", 418, &headers);
    assert_eq!(error.status(), Some(418));
    assert_eq!(error.to_string(), "boom");
    assert!(
        matches!(&error, VllmError::Api { headers: error_headers, .. } if error_headers.get("x").map(String::as_str) == Some("1"))
    );
}

#[test]
fn the_error_class_carries_the_status_message_and_headers() {
    let headers = Headers::from([(String::from("x"), String::from("1"))]);
    let error = get_error_class("boom", 418, &headers);
    assert_eq!(error.status(), Some(418));
    assert_eq!(error.to_string(), "boom");
    assert!(
        matches!(&error, VllmError::Api { headers: error_headers, .. } if error_headers.get("x").map(String::as_str) == Some("1"))
    );
}

#[test]
fn a_provider_error_keeps_its_status_and_message() {
    let provider = ProviderError::from(ProviderErrorParts {
        status_code: 429,
        status_code_is_synthesized: false,
        message: String::from("slow down"),
        headers: None,
        request: None,
        response: None,
        body: None,
    });
    let error = VllmError::from(provider);
    assert_eq!(error.status(), Some(429));
    assert_eq!(error.to_string(), "slow down");
}
