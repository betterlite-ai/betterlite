#![cfg(test)]

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::identity::provider_id::ProviderId;
use http::HeaderMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use tokio::runtime::Builder;
use url::Url;

use super::error::ProxyError;
use super::images::{ImageEndpoint, image_headers, image_url};
use super::models::{ModelsRequest, get_models};
use super::responses::{PROVIDER, SUPPORTS_NATIVE_WEBSOCKET, responses_url};
use super::{LITELLM_PROXY_PREFIX, ProxyOptions, map_params, supported_params, transform_request};
use crate::chat_choices::endpoint::{ModelListError, ModelListTransport};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};

fn config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn catalog(models: &[&str]) -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: models.iter().map(|model| (*model).to_owned()).collect(),
        text_models: BTreeSet::new(),
    }
}

fn options(base: Option<&str>, key: Option<&str>, use_litellm_proxy: bool) -> ProxyOptions {
    ProxyOptions {
        api_base: base.map(str::to_owned),
        api_key: key.map(SecretString::from),
        use_litellm_proxy,
    }
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn revealed(key: Option<&SecretString>) -> Option<String> {
    key.map(|key| key.expose_secret().to_owned())
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn supported_params_append_every_completion_name() {
    let params = supported_params(&config(), "gpt-4o-mini", &catalog(&[]));
    assert_eq!(
        params,
        [
            "frequency_penalty",
            "logit_bias",
            "logprobs",
            "top_logprobs",
            "max_tokens",
            "max_completion_tokens",
            "modalities",
            "prediction",
            "n",
            "presence_penalty",
            "seed",
            "stop",
            "stream",
            "stream_options",
            "temperature",
            "top_p",
            "tools",
            "tool_choice",
            "function_call",
            "functions",
            "max_retries",
            "extra_headers",
            "parallel_tool_calls",
            "audio",
            "web_search_options",
            "service_tier",
            "safety_identifier",
            "prompt_cache_key",
            "prompt_cache_retention",
            "store",
            "response_format",
            "user",
            "request_timeout",
            "api_base",
            "api_version",
            "api_key",
            "deployment_id",
            "organization",
            "base_url",
            "default_headers",
            "timeout",
            "response_format",
            "reasoning_effort",
            "thinking",
            "include_server_side_tool_invocations",
            "verbosity",
        ]
    );
}

#[rstest]
#[case::user_outside_the_catalog("gemini/gemini-1.5-pro", false, "user")]
#[case::response_format_for_gpt_4("gpt-4", false, "response_format")]
#[case::user_for_a_catalog_model("gpt-4o", true, "user")]
#[case::deployment_id("anything", false, "deployment_id")]
#[case::reasoning_effort("anything", false, "reasoning_effort")]
#[case::include_server_side_tool_invocations(
    "anything",
    false,
    "include_server_side_tool_invocations"
)]
fn the_union_accepts_a_completion_name(
    #[case] model: &str,
    #[case] in_catalog: bool,
    #[case] name: &str,
) {
    let catalog_models: &[&str] = if in_catalog { &[model] } else { &[] };
    let mut existing = Object::new();
    let requested = object(&format!(r#"{{"{name}": 1}}"#));
    map_params(
        &config(),
        model,
        &requested,
        &mut existing,
        &catalog(catalog_models),
    )
    .unwrap();
    assert_eq!(existing, requested);
}

#[test]
fn an_unsupported_parameter_is_dropped() {
    let mut existing = Object::new();
    map_params(
        &config(),
        "gpt-4o",
        &object(r#"{"not_a_parameter": 1, "extra_body": {"kept": false}}"#),
        &mut existing,
        &catalog(&[]),
    )
    .unwrap();
    assert!(existing.is_empty(), "{existing:?}");
}

#[test]
fn a_supported_parameter_keeps_zero_and_empty_values() {
    let mut existing = Object::new();
    let requested = object(r#"{"temperature": 0, "stop": [], "stream": false, "user": ""}"#);
    map_params(
        &config(),
        "gpt-4o",
        &requested,
        &mut existing,
        &catalog(&[]),
    )
    .unwrap();
    assert_eq!(existing, requested);
}

#[test]
fn an_existing_parameter_is_replaced() {
    let mut existing = object(r#"{"temperature": 0.1, "max_tokens": 5}"#);
    map_params(
        &config(),
        "gpt-4o",
        &object(r#"{"temperature": 0.9}"#),
        &mut existing,
        &catalog(&[]),
    )
    .unwrap();
    assert_eq!(existing, object(r#"{"temperature": 0.9, "max_tokens": 5}"#));
}

#[test]
fn thinking_is_nested_under_extra_body() {
    let mut existing = Object::new();
    map_params(
        &config(),
        "gpt-4o",
        &object(r#"{"thinking": {"type": "enabled", "budget_tokens": 10}}"#),
        &mut existing,
        &catalog(&[]),
    )
    .unwrap();
    assert_eq!(
        existing,
        object(r#"{"extra_body": {"thinking": {"type": "enabled", "budget_tokens": 10}}}"#)
    );
    assert!(!existing.contains_key("thinking"));
}

#[test]
fn thinking_merges_into_an_existing_extra_body() {
    let mut existing = object(r#"{"extra_body": {"caller": 1}, "temperature": 0.2}"#);
    map_params(
        &config(),
        "gpt-4o",
        &object(r#"{"thinking": {"type": "disabled"}}"#),
        &mut existing,
        &catalog(&[]),
    )
    .unwrap();
    assert_eq!(
        existing,
        object(
            r#"{"extra_body": {"caller": 1, "thinking": {"type": "disabled"}}, "temperature": 0.2}"#
        )
    );
}

#[test]
fn thinking_replaces_an_earlier_thinking() {
    let mut existing = object(r#"{"extra_body": {"thinking": {"type": "enabled"}}}"#);
    map_params(
        &config(),
        "gpt-4o",
        &object(r#"{"thinking": {"type": "disabled"}}"#),
        &mut existing,
        &catalog(&[]),
    )
    .unwrap();
    assert_eq!(
        existing,
        object(r#"{"extra_body": {"thinking": {"type": "disabled"}}}"#)
    );
}

#[test]
fn a_null_thinking_is_nested_too() {
    let mut existing = Object::new();
    map_params(
        &config(),
        "gpt-4o",
        &object(r#"{"thinking": null}"#),
        &mut existing,
        &catalog(&[]),
    )
    .unwrap();
    assert_eq!(existing, object(r#"{"extra_body": {"thinking": null}}"#));
}

#[rstest]
#[case::null("null")]
#[case::bool("false")]
#[case::number("1")]
#[case::text("\"kept\"")]
#[case::list("[]")]
fn a_non_object_extra_body_rejects_thinking(#[case] extra_body: &str) {
    let mut existing = object(&format!(r#"{{"extra_body": {extra_body}}}"#));
    let error = map_params(
        &config(),
        "gpt-4o",
        &object(r#"{"thinking": {"type": "enabled"}}"#),
        &mut existing,
        &catalog(&[]),
    )
    .unwrap_err();
    assert!(matches!(error, ProxyError::ExtraBodyNotObject), "{error}");
}

#[rstest]
#[case::strips_the_prefix(LITELLM_PROXY_PREFIX.to_owned() + "gpt-4", "gpt-4")]
#[case::keeps_a_nested_prefix("gemini/gemini-1.5-pro", "gemini/gemini-1.5-pro")]
#[case::strips_only_the_leading_prefix(LITELLM_PROXY_PREFIX.to_owned() + "gemini/gemini-1.5-pro", "gemini/gemini-1.5-pro")]
#[case::leaves_the_bare_prefix_empty(LITELLM_PROXY_PREFIX.to_owned(), "")]
#[case::leaves_a_bare_model("gpt-4", "gpt-4")]
fn route_strips_only_the_leading_proxy_prefix(#[case] model: String, #[case] expected: &str) {
    let route = options(None, None, true).route(&model, None, None);
    assert_eq!(route.model, expected);
    assert_eq!(route.provider, ProviderId::LitellmProxy);
}

#[test]
fn route_prefers_explicit_credentials() {
    let route = options(Some("https://configured"), Some("configured-key"), true).route(
        "gpt-4",
        Some("https://explicit"),
        Some(&SecretString::from("explicit-key")),
    );
    assert_eq!(route.api_base.as_deref(), Some("https://explicit"));
    assert_eq!(
        revealed(route.api_key.as_ref()).as_deref(),
        Some("explicit-key")
    );
}

#[test]
fn route_falls_back_to_configured_credentials() {
    let route = options(Some("https://configured"), Some("configured-key"), true)
        .route("gpt-4", None, None);
    assert_eq!(route.api_base.as_deref(), Some("https://configured"));
    assert_eq!(
        revealed(route.api_key.as_ref()).as_deref(),
        Some("configured-key")
    );
}

#[test]
fn route_skips_an_empty_explicit_credential() {
    let route = options(Some("https://configured"), Some("configured-key"), true).route(
        "gpt-4",
        Some(""),
        Some(&SecretString::from("")),
    );
    assert_eq!(route.api_base.as_deref(), Some("https://configured"));
    assert_eq!(
        revealed(route.api_key.as_ref()).as_deref(),
        Some("configured-key")
    );
}

#[test]
fn route_leaves_credentials_absent_without_a_source() {
    let route = options(None, None, true).route("gpt-4", None, None);
    assert_eq!(route.api_base, None);
    assert_eq!(route.api_key, None);
    assert_eq!(route.model, "gpt-4");
}

#[test]
fn an_empty_configured_credential_is_absent() {
    let route = options(Some(""), Some(""), true).route("gpt-4", None, None);
    assert_eq!(route.api_base, None);
    assert_eq!(route.api_key, None);
}

#[rstest]
#[case::neither(false, false, false)]
#[case::deployment_flag(false, true, true)]
#[case::environment_or_global(true, false, true)]
#[case::both(true, true, true)]
fn should_use_litellm_proxy_or_the_deployment_flag(
    #[case] configured: bool,
    #[case] deployment: bool,
    #[case] expected: bool,
) {
    assert_eq!(
        options(None, None, configured).should_use_litellm_proxy(deployment),
        expected
    );
}

#[test]
fn transform_request_preserves_the_messages_value() {
    let messages =
        value(r#"[{"role": "user", "content": [{"type": "text", "text": "hi"}], "extra": null}]"#);
    let body = transform_request("gpt-4o", messages.clone(), &Object::new());
    assert_eq!(body.get("messages"), Some(&messages));
    assert_eq!(
        body.get("model"),
        Some(&Value::String(String::from("gpt-4o")))
    );
}

#[rstest]
#[case::model("model", r#""replaced""#)]
#[case::messages("messages", "[]")]
#[case::temperature("temperature", "0.5")]
fn optional_params_are_expanded_after_the_fixed_fields(#[case] name: &str, #[case] raw: &str) {
    let optional = Object::from_iter([(String::from(name), value(raw))]);
    let body = transform_request("gpt-4o", value("[]"), &optional);
    assert_eq!(body.get(name), Some(&value(raw)));
    if name != "model" {
        assert_eq!(
            body.get("model"),
            Some(&Value::String(String::from("gpt-4o")))
        );
    }
}

#[test]
fn transform_request_matches_the_inherited_request_body() {
    let messages = value(r#"[{"role": "user", "content": "hi"}]"#);
    let optional = object(r#"{"model": "other", "temperature": 0.4}"#);
    assert_eq!(
        transform_request("gpt-4o", messages.clone(), &optional),
        config().request_body("gpt-4o", messages, &optional)
    );
}

#[rstest]
#[case::trims_every_slash("https://proxy.example///", "https://proxy.example/responses")]
#[case::keeps_a_path("https://proxy.example/v1/", "https://proxy.example/v1/responses")]
#[case::uses_the_configured_base(None, "https://configured.example/responses")]
fn responses_url_trims_trailing_slashes(#[case] explicit: Option<&str>, #[case] expected: &str) {
    let options = options(Some("https://configured.example/"), None, false);
    assert_eq!(responses_url(&options, explicit).unwrap(), expected);
}

#[test]
fn responses_url_requires_a_base() {
    let error = responses_url(&options(None, None, false), None).unwrap_err();
    assert!(matches!(error, ProxyError::MissingApiBase), "{error}");
}

#[test]
fn responses_advertises_no_native_websocket() {
    assert_eq!(PROVIDER, ProviderId::LitellmProxy);
    const { assert!(!SUPPORTS_NATIVE_WEBSOCKET) };
}

#[test]
fn image_headers_render_a_missing_key_as_bearer_none() {
    let headers = image_headers(&options(None, None, false), &Headers::new(), None);
    assert_eq!(
        headers,
        Headers::from([(String::from("Authorization"), String::from("Bearer None"))])
    );
}

#[rstest]
#[case::explicit_key(Some("explicit-key"), None, "Bearer explicit-key")]
#[case::configured_key(None, Some("configured-key"), "Bearer configured-key")]
#[case::explicit_wins(Some("explicit-key"), Some("configured-key"), "Bearer explicit-key")]
fn image_headers_resolve_the_key(
    #[case] explicit: Option<&str>,
    #[case] configured: Option<&str>,
    #[case] expected: &str,
) {
    let headers = image_headers(
        &options(None, configured, false),
        &Headers::new(),
        explicit.map(SecretString::from).as_ref(),
    );
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some(expected)
    );
}

#[test]
fn image_headers_replace_an_authorization_the_caller_sent() {
    let caller = Headers::from([
        (String::from("Authorization"), String::from("Bearer caller")),
        (String::from("X-Trace"), String::from("1")),
    ]);
    let headers = image_headers(&options(None, Some("configured-key"), false), &caller, None);
    assert_eq!(
        headers,
        Headers::from([
            (
                String::from("Authorization"),
                String::from("Bearer configured-key")
            ),
            (String::from("X-Trace"), String::from("1")),
        ])
    );
}

#[rstest]
#[case::generations(ImageEndpoint::Generations, "/images/generations")]
#[case::edits(ImageEndpoint::Edits, "/images/edits")]
fn image_urls_trim_trailing_slashes(#[case] endpoint: ImageEndpoint, #[case] path: &str) {
    let options = options(Some("https://configured.example/base/"), None, false);
    assert_eq!(
        image_url(&options, Some("https://proxy.example///"), endpoint).unwrap(),
        format!("https://proxy.example{path}")
    );
    assert_eq!(
        image_url(&options, None, endpoint).unwrap(),
        format!("https://configured.example/base{path}")
    );
}

#[rstest]
#[case::generations(ImageEndpoint::Generations)]
#[case::edits(ImageEndpoint::Edits)]
fn image_urls_require_a_base(#[case] endpoint: ImageEndpoint) {
    let error = image_url(&options(None, None, false), None, endpoint).unwrap_err();
    assert!(matches!(error, ProxyError::MissingApiBase), "{error}");
}

const MODELS: &str = r#"{"data": [{"id": "b"}, {"id": "a"}]}"#;

struct Script {
    status: u16,
    body: String,
    requests: Mutex<Vec<(String, Option<String>)>>,
}

impl Script {
    fn replying(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requested(&self) -> Vec<(String, Option<String>)> {
        self.requests.lock().unwrap().clone()
    }
}

impl ModelListTransport for Script {
    async fn get(
        &self,
        url: &Url,
        headers: &HeaderMap,
        _deadline: Duration,
    ) -> Result<UpstreamResponse, TransportError> {
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        self.requests
            .lock()
            .unwrap()
            .push((url.to_string(), authorization));
        Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone().into_bytes(),
        })
    }
}

fn models_request<'request>(
    api_base: Option<&'request str>,
    api_key: Option<&'request SecretString>,
    environment_key: Option<&str>,
) -> ModelsRequest<'request> {
    ModelsRequest {
        api_base,
        api_key,
        environment_api_key: environment_key.map(SecretString::from),
        deadline: Duration::from_secs(600),
    }
}

fn listed(options: &ProxyOptions, script: &Script, request: &ModelsRequest<'_>) -> Vec<String> {
    block_on(get_models(options, script, request)).unwrap()
}

#[test]
fn get_models_prefixes_every_listed_id() {
    let script = Script::replying(200, MODELS);
    let key = SecretString::from("explicit-key");
    let models = listed(
        &options(None, None, true),
        &script,
        &models_request(
            Some("https://proxy.example/v1"),
            Some(&key),
            Some("environment-key"),
        ),
    );
    assert_eq!(models, ["litellm_proxy/b", "litellm_proxy/a"]);
    assert_eq!(
        script.requested(),
        [(
            String::from("https://proxy.example/v1/models"),
            Some(String::from("Bearer explicit-key"))
        )]
    );
}

#[test]
fn get_models_uses_the_configured_base_and_key() {
    let script = Script::replying(200, MODELS);
    let models = listed(
        &options(
            Some("https://configured.example"),
            Some("configured-key"),
            true,
        ),
        &script,
        &models_request(None, None, Some("environment-key")),
    );
    assert_eq!(models, ["litellm_proxy/b", "litellm_proxy/a"]);
    assert_eq!(
        script.requested(),
        [(
            String::from("https://configured.example/v1/models"),
            Some(String::from("Bearer configured-key"))
        )]
    );
}

#[test]
fn get_models_falls_back_to_the_environment_key() {
    let script = Script::replying(200, MODELS);
    let models = listed(
        &options(Some("https://configured.example"), None, true),
        &script,
        &models_request(None, None, Some("environment-key")),
    );
    assert_eq!(models, ["litellm_proxy/b", "litellm_proxy/a"]);
    assert_eq!(
        script.requested(),
        [(
            String::from("https://configured.example/v1/models"),
            Some(String::from("Bearer environment-key"))
        )]
    );
}

#[test]
fn get_models_requires_a_base() {
    let script = Script::replying(200, MODELS);
    let error = block_on(get_models(
        &options(None, None, true),
        &script,
        &models_request(None, None, None),
    ))
    .unwrap_err();
    assert!(matches!(error, ProxyError::MissingApiBase), "{error}");
    assert!(script.requested().is_empty());
}

#[test]
fn get_models_reports_a_status_failure() {
    let script = Script::replying(500, MODELS);
    let error = block_on(get_models(
        &options(Some("https://configured.example"), None, true),
        &script,
        &models_request(None, None, None),
    ))
    .unwrap_err();
    assert!(
        matches!(
            error,
            ProxyError::ModelList {
                source: ModelListError::Status { status: 500 }
            }
        ),
        "{error}"
    );
}

#[test]
fn get_models_reports_a_decode_failure() {
    let script = Script::replying(200, "not json");
    let error = block_on(get_models(
        &options(Some("https://configured.example"), None, true),
        &script,
        &models_request(None, None, None),
    ))
    .unwrap_err();
    assert!(
        matches!(
            error,
            ProxyError::ModelList {
                source: ModelListError::Decode { .. }
            }
        ),
        "{error}"
    );
}
