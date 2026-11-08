use http::header::{ACCEPT, HeaderMap, HeaderValue};
use rstest::rstest;
use secrecy::ExposeSecret;
use serde_json::{Value, json};

use super::support::{environment, object, params, secret};
use crate::watsonx_auth::credentials::{get_api_params, get_watsonx_credentials};
use crate::watsonx_auth::error::WatsonxError;
use crate::watsonx_auth::payload::prepare_payload;
use crate::watsonx_auth::types::{
    DEFAULT_API_VERSION, WatsonxApiParams, WatsonxAudioRequestFields, WatsonxEndpoint,
};
use crate::watsonx_auth::urls::{add_api_version_to_url, get_base_url};

#[test]
fn api_params_pop_both_nested_keys_and_prefer_the_outer() {
    let mut optional_params = params(&[
        ("project_id", json!("outer-project")),
        ("watsonx_project", json!("inner-project")),
        ("space_id", json!("space")),
        ("region_name", json!("outer-region")),
        ("region", json!("inner-region")),
        ("watsonx_region_name", json!("third-region")),
        ("watsonx_region", json!("fourth-region")),
        ("unrelated", json!("kept")),
    ]);
    let found = get_api_params(&mut optional_params, None, &environment(&[])).unwrap();
    assert_eq!(
        found,
        WatsonxApiParams {
            project_id: Some("outer-project".to_owned()),
            space_id: Some("space".to_owned()),
            region_name: Some("outer-region".to_owned()),
        }
    );
    assert_eq!(
        optional_params,
        params(&[
            ("watsonx_region_name", json!("third-region")),
            ("watsonx_region", json!("fourth-region")),
            ("unrelated", json!("kept")),
        ])
    );
}

#[test]
fn api_params_fall_to_the_nested_keys_when_the_outer_is_absent() {
    let mut optional_params = params(&[
        ("watsonx_project", json!("inner-project")),
        ("region", json!("inner-region")),
    ]);
    let found = get_api_params(&mut optional_params, None, &environment(&[])).unwrap();
    assert_eq!(
        found,
        WatsonxApiParams {
            project_id: Some("inner-project".to_owned()),
            space_id: None,
            region_name: Some("inner-region".to_owned()),
        }
    );
    assert!(optional_params.is_empty());
}

#[test]
fn api_params_pop_the_region_pair_when_the_outer_region_is_null() {
    let mut optional_params = params(&[
        ("region_name", Value::Null),
        ("region", json!("inner-region")),
        ("watsonx_region_name", json!("third-region")),
        ("watsonx_region", json!("fourth-region")),
    ]);
    let found = get_api_params(&mut optional_params, None, &environment(&[])).unwrap();
    assert_eq!(found.region_name, Some("third-region".to_owned()));
    assert!(optional_params.is_empty());
}

#[test]
fn api_params_keep_an_explicit_empty_project_id() {
    let mut optional_params = params(&[("project_id", json!("")), ("space_id", json!("space"))]);
    let found = get_api_params(&mut optional_params, None, &environment(&[])).unwrap();
    assert_eq!(found.project_id, Some(String::new()));
    assert_eq!(found.space_id, Some("space".to_owned()));
}

#[test]
fn api_params_fill_missing_values_from_the_environment_in_order() {
    let values = [
        ("WATSONX_PROJECT_ID", "first-project"),
        ("WX_PROJECT_ID", "second-project"),
        ("PROJECT_ID", "third-project"),
        ("WATSONX_REGION", "first-region"),
        ("WX_REGION", "second-region"),
        ("REGION", "third-region"),
        ("WATSONX_DEPLOYMENT_SPACE_ID", "first-space"),
        ("WATSONX_SPACE_ID", "second-space"),
        ("WX_SPACE_ID", "third-space"),
        ("SPACE_ID", "fourth-space"),
    ];
    let found = get_api_params(&mut params(&[]), None, &environment(&values)).unwrap();
    assert_eq!(
        found,
        WatsonxApiParams {
            project_id: Some("first-project".to_owned()),
            space_id: Some("first-space".to_owned()),
            region_name: Some("first-region".to_owned()),
        }
    );
}

#[test]
fn api_params_skip_empty_environment_values() {
    let values = [
        ("WATSONX_PROJECT_ID", ""),
        ("WX_PROJECT_ID", "second-project"),
        ("WATSONX_REGION", ""),
        ("WX_REGION", "second-region"),
        ("WATSONX_DEPLOYMENT_SPACE_ID", ""),
        ("WATSONX_SPACE_ID", "second-space"),
    ];
    let found = get_api_params(&mut params(&[]), None, &environment(&values)).unwrap();
    assert_eq!(found.project_id, Some("second-project".to_owned()));
    assert_eq!(found.region_name, Some("second-region".to_owned()));
    assert_eq!(found.space_id, Some("second-space".to_owned()));
}

#[rstest]
#[case::plain_model(Some("granite-13b"), true)]
#[case::deployment_model(Some("deployment/abc"), false)]
#[case::no_model(None, false)]
fn api_params_require_ids_only_for_non_deployment_models(
    #[case] model: Option<&str>,
    #[case] fails: bool,
) {
    let mut optional_params = params(&[]);
    let found = get_api_params(&mut optional_params, model, &environment(&[]));
    if fails {
        let Err(WatsonxError::Api {
            status_code,
            message,
            headers,
        }) = found
        else {
            panic!("model {model:?} must require ids");
        };
        assert_eq!(status_code, 401);
        assert_eq!(
            message,
            "Error: Watsonx project_id and space_id not set. Set WX_PROJECT_ID or WX_SPACE_ID in environment variables or pass in as parameter."
        );
        assert_eq!(headers, None);
    } else {
        found.unwrap();
    }
}

#[test]
fn credentials_wx_credentials_override_the_arguments() {
    let mut optional_params = params(&[
        ("apikey", json!("param-key")),
        ("url", json!("https://param-url")),
        (
            "wx_credentials",
            object(&[
                ("url", json!("https://wx-url")),
                ("apikey", json!("wx-key")),
                ("token", json!("wx-token")),
            ]),
        ),
    ]);
    let argument_key = secret("argument-key");
    let found = get_watsonx_credentials(
        &mut optional_params,
        Some(&argument_key),
        Some("https://argument-base"),
        &environment(&[]),
    )
    .unwrap();
    assert_eq!(found.api_key.expose_secret(), "wx-key");
    assert_eq!(found.api_base, "https://wx-url");
    assert_eq!(
        found.token.as_ref().map(ExposeSecret::expose_secret),
        Some("wx-token")
    );
    assert_eq!(
        optional_params,
        params(&[
            ("apikey", json!("param-key")),
            ("url", json!("https://param-url")),
        ])
    );
}

#[test]
fn credentials_pop_the_base_chain_and_keep_the_first() {
    let mut optional_params = params(&[
        ("apikey", json!("key")),
        ("base_url", json!("https://base-url")),
        ("api_base", json!("https://api-base")),
        ("url", json!("https://url")),
    ]);
    let found =
        get_watsonx_credentials(&mut optional_params, None, None, &environment(&[])).unwrap();
    assert_eq!(found.api_key.expose_secret(), "key");
    assert_eq!(found.api_base, "https://url");
    assert!(optional_params.is_empty());
}

#[test]
fn credentials_use_the_later_base_name_when_earlier_ones_are_absent() {
    let mut optional_params = params(&[
        ("apikey", json!("key")),
        ("base_url", json!("https://base-url")),
    ]);
    let found =
        get_watsonx_credentials(&mut optional_params, None, None, &environment(&[])).unwrap();
    assert_eq!(found.api_base, "https://base-url");
}

#[test]
fn credentials_skip_the_pops_when_the_arguments_are_set() {
    let mut optional_params = params(&[
        ("apikey", json!("param-key")),
        ("url", json!("https://url")),
    ]);
    let argument_key = secret("argument-key");
    let found = get_watsonx_credentials(
        &mut optional_params,
        Some(&argument_key),
        Some("https://argument-base"),
        &environment(&[]),
    )
    .unwrap();
    assert_eq!(found.api_key.expose_secret(), "argument-key");
    assert_eq!(found.api_base, "https://argument-base");
    assert_eq!(optional_params.len(), 2);
}

#[test]
fn credentials_fall_back_to_watsonx_credentials() {
    let mut optional_params = params(&[(
        "watsonx_credentials",
        object(&[
            ("api_key", json!("alternate-key")),
            ("watsonx_token", json!("alternate-token")),
        ]),
    )]);
    let found = get_watsonx_credentials(
        &mut optional_params,
        None,
        Some("https://argument-base"),
        &environment(&[]),
    )
    .unwrap();
    assert_eq!(found.api_key.expose_secret(), "alternate-key");
    assert_eq!(
        found.token.as_ref().map(ExposeSecret::expose_secret),
        Some("alternate-token")
    );
    assert!(optional_params.is_empty());
}

#[test]
fn credentials_ignore_a_null_wx_credentials_and_drop_watsonx_credentials() {
    let mut optional_params = params(&[
        ("apikey", json!("key")),
        ("wx_credentials", Value::Null),
        (
            "watsonx_credentials",
            object(&[("apikey", json!("alternate-key"))]),
        ),
    ]);
    let found = get_watsonx_credentials(
        &mut optional_params,
        None,
        Some("https://argument-base"),
        &environment(&[]),
    )
    .unwrap();
    assert_eq!(found.api_key.expose_secret(), "key");
    assert!(optional_params.is_empty());
}

#[test]
fn credentials_pop_both_credential_keys_and_prefer_wx_credentials() {
    let mut optional_params = params(&[
        ("wx_credentials", object(&[("apikey", json!("wx-key"))])),
        (
            "watsonx_credentials",
            object(&[("apikey", json!("alternate-key"))]),
        ),
    ]);
    let found = get_watsonx_credentials(
        &mut optional_params,
        None,
        Some("https://argument-base"),
        &environment(&[]),
    )
    .unwrap();
    assert_eq!(found.api_key.expose_secret(), "wx-key");
    assert!(optional_params.is_empty());
}

#[test]
fn credentials_take_the_environment_for_null_key_and_base() {
    let mut optional_params = params(&[("apikey", Value::Null), ("url", Value::Null)]);
    let values = [
        ("WATSONX_APIKEY", "env-key"),
        ("WATSONX_API_BASE", "https://env-base"),
    ];
    let found =
        get_watsonx_credentials(&mut optional_params, None, None, &environment(&values)).unwrap();
    assert_eq!(found.api_key.expose_secret(), "env-key");
    assert_eq!(found.api_base, "https://env-base");
}

#[test]
fn credentials_read_the_key_and_base_from_the_environment() {
    let values = [
        ("WATSONX_APIKEY", "first-key"),
        ("WATSONX_API_KEY", "second-key"),
        ("WX_API_KEY", "third-key"),
        ("WATSONX_ZENAPIKEY", "fourth-key"),
        ("WATSONX_API_BASE", "https://api-base"),
        ("WATSONX_URL", "https://url"),
        ("WX_URL", "https://wx-url"),
        ("WML_URL", "https://wml-url"),
    ];
    let found =
        get_watsonx_credentials(&mut params(&[]), None, None, &environment(&values)).unwrap();
    assert_eq!(found.api_key.expose_secret(), "first-key");
    assert_eq!(found.api_base, "https://api-base");
}

#[test]
fn credentials_skip_empty_environment_values() {
    let values = [
        ("WATSONX_APIKEY", ""),
        ("WATSONX_API_KEY", "second-key"),
        ("WATSONX_API_BASE", ""),
        ("WATSONX_URL", "https://url"),
    ];
    let found =
        get_watsonx_credentials(&mut params(&[]), None, None, &environment(&values)).unwrap();
    assert_eq!(found.api_key.expose_secret(), "second-key");
    assert_eq!(found.api_base, "https://url");
}

#[test]
fn credentials_require_a_key_and_a_base_with_the_source_messages() {
    let missing_key =
        get_watsonx_credentials(&mut params(&[]), None, None, &environment(&[])).unwrap_err();
    let WatsonxError::Api {
        status_code,
        message,
        headers,
    } = missing_key
    else {
        panic!("expected the 401 API key error");
    };
    assert_eq!(status_code, 401);
    assert_eq!(
        message,
        "Error: Watsonx API key not set. Set WATSONX_API_KEY in environment variables or pass in as parameter - 'api_key='."
    );
    assert_eq!(headers, None);

    let mut optional_params = params(&[("apikey", json!("key"))]);
    let missing_base =
        get_watsonx_credentials(&mut optional_params, None, None, &environment(&[])).unwrap_err();
    let WatsonxError::Api {
        status_code: base_status,
        message: base_message,
        ..
    } = missing_base
    else {
        panic!("expected the 401 API base error");
    };
    assert_eq!(base_status, 401);
    assert_eq!(
        base_message,
        "Error: Watsonx API base not set. Set WATSONX_API_BASE in environment variables or pass in as parameter - 'api_base='."
    );
}

#[test]
fn credentials_treat_a_null_url_override_as_a_missing_base() {
    let mut optional_params = params(&[
        ("apikey", json!("key")),
        ("wx_credentials", object(&[("url", Value::Null)])),
    ]);
    let error =
        get_watsonx_credentials(&mut optional_params, None, None, &environment(&[])).unwrap_err();
    assert!(matches!(
        error,
        WatsonxError::Api {
            status_code: 401,
            ..
        }
    ));
}

#[test]
fn base_url_prefers_the_argument_then_the_environment() {
    let values = [
        ("WATSONX_API_BASE", "https://api-base"),
        ("WATSONX_URL", "https://url"),
        ("WX_URL", "https://wx-url"),
        ("WML_URL", "https://wml-url"),
    ];
    assert_eq!(
        get_base_url(Some("https://argument"), &environment(&values)).unwrap(),
        "https://argument"
    );
    assert_eq!(
        get_base_url(Some(""), &environment(&values)).unwrap(),
        "https://api-base"
    );
    assert_eq!(
        get_base_url(None, &environment(&values)).unwrap(),
        "https://api-base"
    );
    let later = [("WX_URL", "https://wx-url"), ("WML_URL", "https://wml-url")];
    assert_eq!(
        get_base_url(None, &environment(&later)).unwrap(),
        "https://wx-url"
    );
}

#[test]
fn base_url_without_a_value_raises_401() {
    let error = get_base_url(None, &environment(&[])).unwrap_err();
    let WatsonxError::Api {
        status_code,
        message,
        ..
    } = error
    else {
        panic!("expected the 401 URL error");
    };
    assert_eq!(status_code, 401);
    assert_eq!(
        message,
        "Error: Watsonx URL not set. Set WATSONX_API_BASE in environment variables or pass in as parameter - 'api_base='."
    );
}

#[rstest]
#[case::no_version(
    "https://x/ml/v1/text/chat",
    None,
    "https://x/ml/v1/text/chat?version=2024-03-13"
)]
#[case::explicit(
    "https://x/ml/v1/text/chat",
    Some("2024-05-01"),
    "https://x/ml/v1/text/chat?version=2024-05-01"
)]
#[case::empty(
    "https://x/ml/v1/text/chat",
    Some(""),
    "https://x/ml/v1/text/chat?version=2024-03-13"
)]
#[case::query_kept("https://x/p?q=1", None, "https://x/p?q=1?version=2024-03-13")]
fn api_version_is_always_appended(
    #[case] url: &str,
    #[case] version: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(add_api_version_to_url(url, version), expected);
}

#[test]
fn default_api_version_is_the_pinned_value() {
    assert_eq!(DEFAULT_API_VERSION, "2024-03-13");
}

#[rstest]
#[case::deployment("deployment/abc", Some("project"), Some("space"), object(&[]))]
#[case::project("granite-13b", Some("project"), Some("space"), object(&[("model_id", json!("granite-13b")), ("project_id", json!("project"))]))]
#[case::space("granite-13b", None, Some("space"), object(&[("model_id", json!("granite-13b")), ("space_id", json!("space"))]))]
#[case::missing_space("granite-13b", None, None, object(&[("model_id", json!("granite-13b")), ("space_id", Value::Null)]))]
fn prepare_payload_uses_the_project_or_the_space(
    #[case] model: &str,
    #[case] project_id: Option<&str>,
    #[case] space_id: Option<&str>,
    #[case] expected: Value,
) {
    let api_params = WatsonxApiParams {
        project_id: project_id.map(str::to_owned),
        space_id: space_id.map(str::to_owned),
        region_name: None,
    };
    assert_eq!(
        serde_json::to_value(prepare_payload(model, &api_params)).unwrap(),
        expected
    );
}

#[rstest]
#[case::text_generation(WatsonxEndpoint::TextGeneration, "/ml/v1/text/generation")]
#[case::text_generation_stream(
    WatsonxEndpoint::TextGenerationStream,
    "/ml/v1/text/generation_stream"
)]
#[case::chat(WatsonxEndpoint::Chat, "/ml/v1/text/chat")]
#[case::chat_stream(WatsonxEndpoint::ChatStream, "/ml/v1/text/chat_stream")]
#[case::deployment_text_generation(
    WatsonxEndpoint::DeploymentTextGeneration,
    "/ml/v1/deployments/{deployment_id}/text/generation"
)]
#[case::deployment_text_generation_stream(
    WatsonxEndpoint::DeploymentTextGenerationStream,
    "/ml/v1/deployments/{deployment_id}/text/generation_stream"
)]
#[case::deployment_chat(
    WatsonxEndpoint::DeploymentChat,
    "/ml/v1/deployments/{deployment_id}/text/chat"
)]
#[case::deployment_chat_stream(
    WatsonxEndpoint::DeploymentChatStream,
    "/ml/v1/deployments/{deployment_id}/text/chat_stream"
)]
#[case::embeddings(WatsonxEndpoint::Embeddings, "/ml/v1/text/embeddings")]
#[case::prompts(WatsonxEndpoint::Prompts, "/ml/v1/prompts")]
#[case::available_models(WatsonxEndpoint::AvailableModels, "/ml/v1/foundation_model_specs")]
#[case::rerank(WatsonxEndpoint::Rerank, "/ml/v1/text/rerank")]
fn endpoint_paths_match_the_source(#[case] endpoint: WatsonxEndpoint, #[case] path: &str) {
    assert_eq!(endpoint.path(), path);
}

#[test]
fn audio_request_fields_omit_unset_options() {
    let fields = WatsonxAudioRequestFields {
        model: "whisper-large-v3-turbo".to_owned(),
        project_id: None,
        space_id: None,
        language: None,
        prompt: None,
        response_format: None,
        temperature: None,
        timestamp_granularities: None,
    };
    assert_eq!(
        serde_json::to_value(&fields).unwrap(),
        object(&[("model", json!("whisper-large-v3-turbo"))])
    );
    let decoded: WatsonxAudioRequestFields = serde_json::from_value(object(&[
        ("model", json!("whisper-large-v3-turbo")),
        ("project_id", json!("project")),
        ("language", json!("en")),
        ("temperature", json!(0.5_f64)),
        ("timestamp_granularities", json!(["word", "segment"])),
    ]))
    .unwrap();
    assert_eq!(decoded.project_id.as_deref(), Some("project"));
    assert_eq!(decoded.temperature, Some(0.5_f64));
    assert_eq!(
        decoded.timestamp_granularities,
        Some(vec!["word".to_owned(), "segment".to_owned()])
    );
}

#[test]
fn watsonx_error_carries_status_message_and_headers() {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    let error = WatsonxError::Api {
        status_code: 503,
        message: "unavailable".to_owned(),
        headers: Some(headers.clone()),
    };
    let WatsonxError::Api {
        status_code,
        message,
        headers: found,
    } = error
    else {
        panic!("expected the Watsonx API error");
    };
    assert_eq!(status_code, 503);
    assert_eq!(message, "unavailable");
    assert_eq!(found, Some(headers));
}
