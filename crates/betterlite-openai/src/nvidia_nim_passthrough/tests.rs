#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::{
    NvidiaNimPassthroughConfig, NvidiaNimPassthroughError, validate_environment,
    without_repeated_version_prefix,
};
use crate::openai_base_config::{Headers, Object};
use crate::passthrough::transformation::types::{
    PassthroughError, PassthroughLogging, PassthroughTarget, PassthroughUrlRequest, RelayCallType,
    RelayedBody, RelayedResponse,
};
use crate::passthrough::transformation::BasePassthroughConfig;

fn object(value: Value) -> Object {
    match value {
        Value::Object(fields) => fields,
        _ => panic!("the fixture is not an object"),
    }
}

fn target(model: &'static str) -> PassthroughTarget<'static> {
    PassthroughTarget {
        model,
        custom_llm_provider: "nvidia_nim",
        endpoint: "chat/completions",
    }
}

fn params(model_group: Value) -> Object {
    object(json!({"litellm_metadata": {"model_group": model_group}}))
}

fn url_request<'call>(
    api_base: Option<&'call str>,
    model: &'call str,
    endpoint: &'call str,
    litellm_params: &'call Object,
) -> PassthroughUrlRequest<'call> {
    PassthroughUrlRequest {
        api_base,
        api_key: None,
        target: PassthroughTarget {
            model,
            custom_llm_provider: "nvidia_nim",
            endpoint,
        },
        request_query_params: None,
        litellm_params,
    }
}

#[rstest]
#[case::repeats_the_last_segment("https://h/v1", "v1/chat/completions", "https://h")]
#[case::keeps_earlier_segments("https://h/api/v1", "/v1/models", "https://h/api")]
#[case::trailing_slash("https://h/v1/", "v1/models", "https://h")]
#[case::other_version_kept("https://h/v1", "v2/models", "https://h/v1")]
#[case::no_version_kept("https://h/v1", "chat/completions", "https://h/v1")]
#[case::empty_base_path("https://h", "v1/models", "https://h")]
#[case::query_dropped("https://h/v1?key=1", "v1/models", "https://h")]
#[case::case_sensitive("https://h/api/V1", "v1/models", "https://h/api/V1")]
#[case::leading_zero_is_a_version("https://h/v01", "v01/models", "https://h")]
#[case::bare_v_is_not_a_version("https://h/v", "v/models", "https://h/v")]
#[case::digits_only_after_v("https://h/v1x", "v1x/models", "https://h/v1x")]
#[case::fragment_kept("https://h/v1#part", "v1/models", "https://h/#part")]
fn without_repeated_version_prefix_drops_one_repeated_version(
    #[case] api_base: &str,
    #[case] native_endpoint: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        without_repeated_version_prefix(api_base, native_endpoint).unwrap(),
        expected
    );
}

#[test]
fn without_repeated_version_prefix_rejects_an_invalid_base() {
    let error = without_repeated_version_prefix("not a url", "v1/models").unwrap_err();
    assert!(matches!(
        error,
        NvidiaNimPassthroughError::Passthrough(PassthroughError::InvalidUrl { .. })
    ));
}

#[test]
fn get_api_base_prefers_the_argument() {
    let config = NvidiaNimPassthroughConfig {
        api_base_env: Some(String::from("https://env")),
        api_key_env: None,
    };
    assert_eq!(
        config.get_api_base(Some("https://argument")),
        Some("https://argument")
    );
    assert_eq!(config.get_api_base(None), Some("https://env"));
    assert_eq!(config.get_api_base(Some("")), Some("https://env"));
}

#[test]
fn get_api_base_treats_an_empty_secret_as_unset() {
    let config = NvidiaNimPassthroughConfig {
        api_base_env: Some(String::new()),
        api_key_env: None,
    };
    assert_eq!(config.get_api_base(None), None);
    assert_eq!(config.get_api_base(Some("")), None);
}

#[test]
fn get_api_key_prefers_the_argument() {
    let env_key = SecretString::from("env-key");
    let config = NvidiaNimPassthroughConfig {
        api_base_env: None,
        api_key_env: Some(env_key),
    };
    let argument = SecretString::from("argument-key");
    assert_eq!(
        config.get_api_key(Some(&argument)).map(|key| key.expose_secret()),
        Some("argument-key")
    );
    assert_eq!(
        config.get_api_key(None).map(|key| key.expose_secret()),
        Some("env-key")
    );
}

#[test]
fn get_api_key_treats_an_empty_key_as_unset() {
    let config = NvidiaNimPassthroughConfig {
        api_base_env: None,
        api_key_env: Some(SecretString::from("")),
    };
    let empty = SecretString::from("");
    assert!(config.get_api_key(Some(&empty)).is_none());
    assert!(config.get_api_key(None).is_none());
}

#[test]
fn validate_environment_returns_the_headers_when_no_key_is_given() {
    let headers = Headers::from([(String::from("Authorization"), String::from("caller"))]);
    assert_eq!(validate_environment(headers.clone(), None), headers);
}

#[test]
fn validate_environment_sets_the_bearer_token_and_keeps_other_headers() {
    let headers = Headers::from([
        (String::from("Authorization"), String::from("caller")),
        (String::from("x-other"), String::from("kept")),
    ]);
    let merged = validate_environment(headers, Some(&SecretString::from("secret")));
    assert_eq!(
        merged,
        Headers::from([
            (String::from("Authorization"), String::from("Bearer secret")),
            (String::from("x-other"), String::from("kept")),
        ])
    );
}

#[rstest]
#[case::true_value(json!({"stream": true}), true)]
#[case::false_value(json!({"stream": false}), false)]
#[case::absent(json!({}), false)]
#[case::non_bool(json!({"stream": 1}), false)]
fn is_streaming_request_reads_the_stream_flag(#[case] request_data: Value, #[case] expected: bool) {
    let config = NvidiaNimPassthroughConfig::default();
    assert_eq!(
        config.is_streaming_request("chat/completions", &object(request_data)),
        expected
    );
}

#[test]
fn get_complete_url_joins_the_root_and_the_native_endpoint() {
    let config = NvidiaNimPassthroughConfig::default();
    let litellm_params = params(json!("group"));
    let request = url_request(
        Some("https://h/v1"),
        "meta/llama",
        "v1/chat/completions",
        &litellm_params,
    );
    let (url, root) = config.get_complete_url(&request).unwrap();
    assert_eq!(url.as_str(), "https://h/v1/chat/completions");
    assert_eq!(root, "https://h");
}

#[test]
fn get_complete_url_strips_the_model_and_model_group_segments() {
    let config = NvidiaNimPassthroughConfig::default();
    let litellm_params = params(json!("group/model"));
    let model_request = url_request(
        Some("https://h"),
        "group/model",
        "group/model/v1/chat/completions",
        &litellm_params,
    );
    assert_eq!(
        config.get_complete_url(&model_request).unwrap().0.as_str(),
        "https://h/v1/chat/completions"
    );
    let group_request = url_request(
        Some("https://h"),
        "meta/llama",
        "group/model/v1/chat/completions",
        &litellm_params,
    );
    assert_eq!(
        config.get_complete_url(&group_request).unwrap().0.as_str(),
        "https://h/v1/chat/completions"
    );
}

#[test]
fn get_complete_url_uses_the_secret_base_when_the_request_has_none() {
    let config = NvidiaNimPassthroughConfig {
        api_base_env: Some(String::from("https://env/v1")),
        api_key_env: None,
    };
    let litellm_params = Object::new();
    let request = url_request(None, "meta/llama", "v1/chat/completions", &litellm_params);
    assert_eq!(
        config.get_complete_url(&request).unwrap().0.as_str(),
        "https://env/v1/chat/completions"
    );
}

#[test]
fn get_complete_url_fails_without_a_base() {
    let config = NvidiaNimPassthroughConfig::default();
    let litellm_params = Object::new();
    let request = url_request(None, "meta/llama", "v1/chat/completions", &litellm_params);
    let error = config.get_complete_url(&request).unwrap_err();
    assert!(matches!(error, NvidiaNimPassthroughError::MissingApiBase));
}

#[test]
fn get_complete_url_applies_the_request_query() {
    let config = NvidiaNimPassthroughConfig::default();
    let litellm_params = Object::new();
    let query = object(json!({"version": "2"}));
    let request = PassthroughUrlRequest {
        api_base: Some("https://h"),
        api_key: None,
        target: target("meta/llama"),
        request_query_params: Some(&query),
        litellm_params: &litellm_params,
    };
    assert_eq!(
        config.get_complete_url(&request).unwrap().0.as_str(),
        "https://h/chat/completions?version=2"
    );
}

fn relayed(body: &[u8]) -> Option<RelayedBody> {
    let config = NvidiaNimPassthroughConfig::default();
    let mut logging = NoLogging;
    config.logging_non_streaming_response(
        &target("meta/llama"),
        &RelayedResponse {
            status_code: 200,
            body,
        },
        &Object::new(),
        &mut logging,
    )
}

struct NoLogging;

impl PassthroughLogging for NoLogging {
    fn set_call_type(&mut self, _call_type: RelayCallType) {}
}

#[test]
fn logging_non_streaming_response_relays_a_json_object() {
    let body = relayed(br#"{"id": "reply"}"#).unwrap();
    assert_eq!(
        body,
        RelayedBody::Json(object(json!({"id": "reply"})))
    );
}

#[test]
fn logging_non_streaming_response_relays_text() {
    let body = relayed(b"not json").unwrap();
    assert_eq!(body, RelayedBody::Text(String::from("not json")));
}

#[test]
fn logging_non_streaming_response_skips_a_body_that_is_not_utf8() {
    assert_eq!(relayed(&[0xff, 0xfe]), None);
}

#[test]
fn get_error_class_carries_the_status_message_and_headers() {
    let config = NvidiaNimPassthroughConfig::default();
    let headers = Headers::from([(String::from("retry-after"), String::from("2"))]);
    let error = config.get_error_class("slow down", 429, &headers);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "slow down");
    assert_eq!(
        error.headers,
        Some(vec![(String::from("retry-after"), String::from("2"))])
    );
}
