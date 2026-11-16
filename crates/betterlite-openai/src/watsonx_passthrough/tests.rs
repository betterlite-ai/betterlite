#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::WatsonxPassthroughConfig;
use crate::openai_base_config::Object;
use crate::passthrough::transformation::BasePassthroughConfig as _;
use crate::passthrough::transformation::types::{PassthroughTarget, PassthroughUrlRequest};
use crate::watsonx_auth::environment::WatsonxEnvironment;

fn environment(values: &[(&str, &str)]) -> WatsonxEnvironment {
    let find = |name: &str| {
        values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    };
    let secret = |name: &str| find(name).map(|value| SecretString::from(value.to_owned()));
    let text = |name: &str| find(name).map(str::to_owned);
    WatsonxEnvironment {
        watsonx_iam_url: text("WATSONX_IAM_URL"),
        wx_api_key: secret("WX_API_KEY"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_apikey: secret("WATSONX_APIKEY"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_url: text("WATSONX_URL"),
        wx_url: text("WX_URL"),
        wml_url: text("WML_URL"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        wx_project_id: text("WX_PROJECT_ID"),
        project_id: text("PROJECT_ID"),
        watsonx_region: text("WATSONX_REGION"),
        wx_region: text("WX_REGION"),
        region: text("REGION"),
        watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
        watsonx_space_id: text("WATSONX_SPACE_ID"),
        wx_space_id: text("WX_SPACE_ID"),
        space_id: text("SPACE_ID"),
    }
}

fn options(wire: &str) -> Object {
    serde_json::from_str(wire).unwrap()
}

fn config(environment: &WatsonxEnvironment) -> WatsonxPassthroughConfig<'_> {
    WatsonxPassthroughConfig { environment }
}

fn url_for<'call>(
    base: Option<&'call str>,
    endpoint: &'call str,
    query: Option<&'call Object>,
    litellm_params: &'call Object,
) -> PassthroughUrlRequest<'call> {
    PassthroughUrlRequest {
        api_base: base,
        api_key: None,
        target: PassthroughTarget {
            model: "m",
            custom_llm_provider: "watsonx",
            endpoint,
        },
        request_query_params: query,
        litellm_params,
    }
}

#[test]
fn url_keeps_base_and_appends_query() {
    let environment = WatsonxEnvironment::default();
    let empty = Object::new();
    let query = options(r#"{"version":"v","x":1}"#);
    let request = url_for(Some("https://host/"), "/v1/chat", Some(&query), &empty);
    let (url, base) = config(&environment).get_complete_url(&request).unwrap();
    assert_eq!(url.as_str(), "https://host/v1/chat?version=v&x=1");
    assert_eq!(base, "https://host/");
}

#[test]
fn url_trims_endpoint_leading_slash() {
    let environment = WatsonxEnvironment::default();
    let empty = Object::new();
    let request = url_for(Some("https://host"), "v1/chat", None, &empty);
    let (url, base) = config(&environment).get_complete_url(&request).unwrap();
    assert_eq!(url.as_str(), "https://host/v1/chat");
    assert_eq!(base, "https://host");
}

#[test]
fn url_falls_back_to_environment_base() {
    let environment = environment(&[("WATSONX_API_BASE", "https://env-host")]);
    let empty = Object::new();
    let request = url_for(None, "v1/chat", None, &empty);
    let (url, base) = config(&environment).get_complete_url(&request).unwrap();
    assert_eq!(url.as_str(), "https://env-host/v1/chat");
    assert_eq!(base, "https://env-host");
}

#[test]
fn url_without_base_is_401() {
    let environment = WatsonxEnvironment::default();
    let empty = Object::new();
    let error = config(&environment)
        .get_complete_url(&url_for(None, "v1/chat", None, &empty))
        .unwrap_err();
    assert_eq!(error.status(), Some(401));
}

#[test]
fn url_with_query_value_that_is_not_text_fails() {
    let environment = WatsonxEnvironment::default();
    let empty = Object::new();
    let query = options(r#"{"version":[1]}"#);
    let error = config(&environment)
        .get_complete_url(&url_for(
            Some("https://host"),
            "v1/chat",
            Some(&query),
            &empty,
        ))
        .unwrap_err();
    assert_eq!(error.status(), None);
}

#[test]
fn url_with_invalid_base_fails() {
    let environment = WatsonxEnvironment::default();
    let empty = Object::new();
    let error = config(&environment)
        .get_complete_url(&url_for(Some("not a url"), "v1/chat", None, &empty))
        .unwrap_err();
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::stream_true(r#"{"stream":true}"#, true)]
#[case::stream_false(r#"{"stream":false}"#, false)]
#[case::stream_missing("{}", false)]
#[case::stream_text(r#"{"stream":"true"}"#, false)]
#[case::stream_null(r#"{"stream":null}"#, false)]
fn streaming_follows_a_boolean_stream_key(#[case] data: &str, #[case] expected: bool) {
    let environment = WatsonxEnvironment::default();
    assert_eq!(
        config(&environment).is_streaming_request("/v1/chat", &options(data)),
        expected
    );
}

#[test]
fn api_key_uses_the_explicit_key() {
    let environment = WatsonxEnvironment::default();
    let key = SecretString::from("given");
    let found = config(&environment).get_api_key(Some(&key)).unwrap();
    assert_eq!(found.expose_secret(), "given");
}

#[test]
fn api_key_falls_back_to_credentials() {
    let environment = environment(&[
        ("WATSONX_API_KEY", "envkey"),
        ("WATSONX_API_BASE", "https://base"),
    ]);
    let found = config(&environment).get_api_key(None).unwrap();
    assert_eq!(found.expose_secret(), "envkey");
}

#[test]
fn api_key_empty_explicit_value_falls_back() {
    let environment = environment(&[
        ("WATSONX_API_KEY", "envkey"),
        ("WATSONX_API_BASE", "https://base"),
    ]);
    let key = SecretString::from("");
    let found = config(&environment).get_api_key(Some(&key)).unwrap();
    assert_eq!(found.expose_secret(), "envkey");
}

#[test]
fn api_key_without_key_is_401() {
    let environment = environment(&[("WATSONX_API_BASE", "https://base")]);
    let error = config(&environment).get_api_key(None).unwrap_err();
    assert_eq!(error.status(), Some(401));
}

// LiteLLM's credentials helper also demands a base, so a key alone cannot be returned
#[test]
fn api_key_without_base_is_401() {
    let environment = environment(&[("WATSONX_API_KEY", "envkey")]);
    let error = config(&environment).get_api_key(None).unwrap_err();
    assert_eq!(error.status(), Some(401));
}
