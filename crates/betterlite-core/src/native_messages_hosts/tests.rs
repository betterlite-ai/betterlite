#![cfg(test)]

use std::future::{Future, ready};

use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::capabilities::reasoning::ReasoningEffortError;
use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_config::sections::anthropic_credentials::DEFAULT_API_BASE;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Number;

use crate::api::anthropic_beta_filter::Headers;
use crate::api::messages_native_transform::capabilities::{
    LegacyShapes, Thinking, ThinkingCapabilities,
};

use super::deepseek;
use super::error::HostError;
use super::keys::{BaseSources, KeySources, resolve_key};
use super::minimax;
use super::request::{EnvironmentRequest, HostRequest, validate_inherited_environment};
use super::tencent;

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn header_map(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::explicit(
    Some("explicit"),
    Some("environment"),
    Some("global"),
    Some("explicit")
)]
#[case::empty_explicit(Some(""), Some("environment"), Some("global"), Some("environment"))]
#[case::empty_environment(Some(""), Some(""), Some("global"), Some("global"))]
#[case::empty_global(Some(""), Some(""), Some(""), Some(""))]
#[case::nothing(None, None, None, None)]
fn keys_follow_the_python_or_chain(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] global: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let explicit_key = explicit.map(secret);
    let environment_key = environment.map(secret);
    let global_key = global.map(secret);
    let resolved = resolve_key(KeySources {
        explicit: explicit_key.as_ref(),
        environment: environment_key.as_ref(),
        global: global_key.as_ref(),
    });
    let exposed = resolved.as_ref().map(ExposeSecret::expose_secret);
    assert_eq!(exposed, expected);
}

#[rstest]
#[case::default(None, None, None, "https://api.deepseek.com/anthropic/v1/messages")]
#[case::trailing_slash(
    Some("https://api.deepseek.com/anthropic/"),
    None,
    None,
    "https://api.deepseek.com/anthropic/v1/messages"
)]
#[case::complete_anthropic(
    Some("https://proxy.test/anthropic/v1/messages"),
    None,
    None,
    "https://proxy.test/anthropic/v1/messages"
)]
#[case::anthropic_v1(
    Some("https://proxy.test/anthropic/v1"),
    None,
    None,
    "https://proxy.test/anthropic/v1/messages"
)]
#[case::beta_suffix(
    Some("https://api.deepseek.com/beta"),
    None,
    None,
    "https://api.deepseek.com/anthropic/v1/messages"
)]
#[case::beta_after_v1(
    Some("https://api.deepseek.com/anthropic/beta/v1"),
    None,
    None,
    "https://api.deepseek.com/anthropic/v1/messages"
)]
#[case::plain_messages(
    Some("https://proxy.test/prefix/v1/messages"),
    None,
    None,
    "https://proxy.test/prefix/anthropic/v1/messages"
)]
#[case::anthropic_inner_path(
    Some("https://proxy.test/anthropic/proxy"),
    None,
    None,
    "https://proxy.test/anthropic/proxy/v1/messages"
)]
#[case::environment(
    Some(""),
    Some("https://env.deepseek.test"),
    Some("https://fallback.deepseek.test"),
    "https://env.deepseek.test/anthropic/v1/messages"
)]
#[case::fallback(
    Some(""),
    Some(""),
    Some("https://fallback.deepseek.test"),
    "https://fallback.deepseek.test/anthropic/v1/messages"
)]
fn deepseek_urls_follow_its_suffix_policy(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] fallback: Option<&str>,
    #[case] expected: &str,
) {
    let sources = BaseSources {
        explicit,
        environment,
        fallback_environment: fallback,
    };
    assert_eq!(deepseek::complete_url(sources), expected);
}

#[rstest]
#[case::default(None, "https://tokenhub-intl.tencentcloudmaas.com/v1/messages")]
#[case::complete(
    Some("https://tokenhub.test/anthropic/v1/messages"),
    "https://tokenhub.test/anthropic/v1/messages"
)]
#[case::chat_completions(
    Some("https://tokenhub.test/v1/chat/completions"),
    "https://tokenhub.test/v1/messages"
)]
#[case::chat_completions_slash(
    Some("https://tokenhub.test/v1/chat/completions/"),
    "https://tokenhub.test/v1/messages"
)]
#[case::v1(Some("https://tokenhub.test/v1"), "https://tokenhub.test/v1/messages")]
#[case::other_version(
    Some("https://tokenhub.test/v2"),
    "https://tokenhub.test/v2/v1/messages"
)]
fn tencent_urls_swap_the_chat_endpoint(#[case] explicit: Option<&str>, #[case] expected: &str) {
    let sources = BaseSources {
        explicit,
        environment: None,
        fallback_environment: None,
    };
    assert_eq!(tencent::complete_url(sources), expected);
}

#[rstest]
#[case::default(None, None, "https://api.minimax.io/anthropic/v1/messages")]
#[case::complete(
    Some("https://api.minimax.io/anthropic/v1/messages"),
    None,
    "https://api.minimax.io/anthropic/v1/messages"
)]
#[case::existing_v1(
    Some("https://api.minimaxi.com/anthropic/v1"),
    None,
    "https://api.minimaxi.com/anthropic/v1/v1/messages"
)]
#[case::trailing_slash(
    Some("https://api.minimaxi.com/anthropic/"),
    None,
    "https://api.minimaxi.com/anthropic/v1/messages"
)]
#[case::bare(
    Some("https://api.minimaxi.com/anthropic"),
    None,
    "https://api.minimaxi.com/anthropic/v1/messages"
)]
#[case::environment(
    Some(""),
    Some("https://gateway.test/minimax/v1/"),
    "https://gateway.test/minimax/v1/v1/messages"
)]
fn minimax_urls_only_adjust_the_separator(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let sources = BaseSources {
        explicit,
        environment,
        fallback_environment: None,
    };
    assert_eq!(minimax::complete_url(sources), expected);
}

#[derive(Default)]
struct Sources {
    explicit: Option<String>,
    environment: Option<String>,
    global: Option<String>,
}

fn deepseek_headers(
    sources: &Sources,
    input: &[(&str, &str)],
    parameters: &IndexMap<String, Json>,
    api_base: Option<&str>,
) -> (Headers, Option<String>) {
    let explicit = sources.explicit.as_deref().map(secret);
    let environment = sources.environment.as_deref().map(secret);
    let global = sources.global.as_deref().map(secret);
    let messages: Vec<Json> = Vec::new();
    let mut headers = header_map(input);
    let request = EnvironmentRequest {
        headers: &mut headers,
        explicit_key: explicit.as_ref(),
        environment_key: environment.as_ref(),
        global_key: global.as_ref(),
        api_base,
        optional_params: parameters,
        messages: &messages,
    };
    let returned_base =
        deepseek::validate_environment(request).expect("deepseek permits a missing key");
    (headers, returned_base)
}

fn inherited_headers(
    sources: &Sources,
    input: &[(&str, &str)],
    parameters: &IndexMap<String, Json>,
    settings: &AnthropicSettings,
) -> Result<Headers, HostError> {
    let explicit = sources.explicit.as_deref().map(secret);
    let environment = sources.environment.as_deref().map(secret);
    let global = sources.global.as_deref().map(secret);
    let messages: Vec<Json> = Vec::new();
    let mut headers = header_map(input);
    let request = EnvironmentRequest {
        headers: &mut headers,
        explicit_key: explicit.as_ref(),
        environment_key: environment.as_ref(),
        global_key: global.as_ref(),
        api_base: None,
        optional_params: parameters,
        messages: &messages,
    };
    validate_inherited_environment(request, settings)?;
    Ok(headers)
}

fn empty_parameters() -> IndexMap<String, Json> {
    IndexMap::new()
}

fn compaction_parameters() -> IndexMap<String, Json> {
    let mut parameters = IndexMap::new();
    parameters.insert("compaction".to_owned(), Json::Object(Object::new()));
    parameters
}

#[test]
fn deepseek_permits_missing_auth_and_adds_its_headers() {
    let sources = Sources::default();
    let (headers, api_base) = deepseek_headers(
        &sources,
        &[],
        &empty_parameters(),
        Some("https://explicit.test"),
    );
    assert_eq!(headers.get("x-api-key"), None);
    assert_eq!(
        headers.get("anthropic-version").map(String::as_str),
        Some("2023-06-01")
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(api_base.as_deref(), Some("https://explicit.test"));
}

#[test]
fn deepseek_inserts_x_api_key_for_a_resolved_key() {
    let sources = Sources {
        explicit: Some("explicit-key".to_owned()),
        environment: None,
        global: None,
    };
    let (headers, _) = deepseek_headers(&sources, &[], &empty_parameters(), None);
    assert_eq!(
        headers.get("x-api-key").map(String::as_str),
        Some("explicit-key")
    );
}

#[test]
fn deepseek_checks_header_names_in_exact_lowercase() {
    let sources = Sources {
        explicit: Some("explicit-key".to_owned()),
        environment: None,
        global: None,
    };
    let (headers, _) = deepseek_headers(
        &sources,
        &[("Authorization", "Bearer supplied")],
        &empty_parameters(),
        None,
    );
    assert_eq!(
        headers.get("x-api-key").map(String::as_str),
        Some("explicit-key")
    );

    let (lowercase_headers, _) = deepseek_headers(
        &sources,
        &[("authorization", "Bearer supplied")],
        &empty_parameters(),
        None,
    );
    assert_eq!(lowercase_headers.get("x-api-key"), None);
}

#[test]
fn deepseek_keeps_supplied_lowercase_headers() {
    let sources = Sources::default();
    let (headers, _) = deepseek_headers(
        &sources,
        &[
            ("anthropic-version", "2024-01-01"),
            ("content-type", "text/plain"),
        ],
        &empty_parameters(),
        None,
    );
    assert_eq!(
        headers.get("anthropic-version").map(String::as_str),
        Some("2024-01-01")
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("text/plain")
    );
}

#[test]
fn deepseek_keeps_an_oauth_key_verbatim_in_x_api_key() {
    let sources = Sources {
        explicit: Some("sk-ant-oat01-token".to_owned()),
        environment: None,
        global: None,
    };
    let (headers, _) = deepseek_headers(&sources, &[], &empty_parameters(), None);
    assert_eq!(
        headers.get("x-api-key").map(String::as_str),
        Some("sk-ant-oat01-token")
    );
    assert_eq!(headers.get("authorization"), None);
}

#[test]
fn deepseek_leaves_an_oauth_authorization_header_alone() {
    let sources = Sources::default();
    let (headers, _) = deepseek_headers(
        &sources,
        &[("authorization", "Bearer sk-ant-oat01-token")],
        &empty_parameters(),
        None,
    );
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some("Bearer sk-ant-oat01-token")
    );
    assert_eq!(headers.get("x-api-key"), None);
    assert_eq!(headers.get("anthropic-beta"), None);
    assert_eq!(
        headers.get("anthropic-dangerous-direct-browser-access"),
        None
    );
}

#[test]
fn deepseek_tags_beta_synthesis_with_its_own_provider() {
    let sources = Sources::default();
    let (headers, _) = deepseek_headers(&sources, &[], &compaction_parameters(), None);
    assert_eq!(headers.get("anthropic-beta"), None);
}

#[test]
fn deepseek_adds_the_structured_outputs_beta() {
    let sources = Sources::default();
    let mut parameters = IndexMap::new();
    parameters.insert("output_format".to_owned(), Json::Object(Object::new()));
    let (headers, _) = deepseek_headers(&sources, &[], &parameters, None);
    assert_eq!(
        headers.get("anthropic-beta").map(String::as_str),
        Some("structured-outputs-2025-11-13")
    );
}

#[test]
fn inherited_hosts_tag_beta_synthesis_as_anthropic() {
    let sources = Sources::default();
    let headers = inherited_headers(
        &sources,
        &[],
        &compaction_parameters(),
        &settings(Some("anthropic-key"), None),
    )
    .expect("the anthropic key authenticates");
    assert_eq!(
        headers.get("anthropic-beta").map(String::as_str),
        Some("compact-2026-09-04")
    );
}

#[test]
fn inherited_hosts_accept_any_header_case() {
    let sources = Sources::default();
    let headers = inherited_headers(
        &sources,
        &[
            ("Authorization", "Bearer supplied"),
            ("X-Api-Key", "supplied"),
        ],
        &empty_parameters(),
        &settings(None, None),
    )
    .expect("the authorization header authenticates");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer supplied")
    );
    assert_eq!(
        headers.get("X-Api-Key").map(String::as_str),
        Some("supplied")
    );
}

#[test]
fn inherited_hosts_report_a_missing_credential() {
    let sources = Sources::default();
    let error = inherited_headers(&sources, &[], &empty_parameters(), &settings(None, None))
        .expect_err("no credential is configured");
    assert_eq!(error.status_code(), Some(401));
    assert_eq!(error.class_name(), "AuthenticationError");
}

#[test]
fn inherited_hosts_fall_back_to_the_anthropic_key() {
    let sources = Sources::default();
    let headers = inherited_headers(
        &sources,
        &[],
        &empty_parameters(),
        &settings(Some("anthropic-key"), None),
    )
    .expect("the anthropic key authenticates");
    assert_eq!(
        headers.get("x-api-key").map(String::as_str),
        Some("anthropic-key")
    );
}

#[test]
fn inherited_hosts_rewrite_an_oauth_key_as_a_bearer_token() {
    let sources = Sources {
        explicit: Some("sk-ant-oat01-token".to_owned()),
        environment: None,
        global: None,
    };
    let headers = inherited_headers(&sources, &[], &empty_parameters(), &settings(None, None))
        .expect("the oauth key authenticates");
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some("Bearer sk-ant-oat01-token")
    );
    assert_eq!(headers.get("x-api-key"), None);
}

#[test]
fn inherited_hosts_keep_the_requested_api_base() {
    let parameters = empty_parameters();
    let messages: Vec<Json> = Vec::new();
    let mut headers = header_map(&[]);
    let request = EnvironmentRequest {
        headers: &mut headers,
        explicit_key: None,
        environment_key: None,
        global_key: None,
        api_base: Some("https://tokenhub.test/v1"),
        optional_params: &parameters,
        messages: &messages,
    };
    let api_base = validate_inherited_environment(request, &settings(Some("anthropic-key"), None))
        .expect("the anthropic key authenticates");
    assert_eq!(api_base.as_deref(), Some("https://tokenhub.test/v1"));
}

fn settings(api_key: Option<&str>, auth_token: Option<&str>) -> AnthropicSettings {
    AnthropicSettings {
        api_key: api_key.map(secret),
        auth_token: auth_token.map(secret),
        api_base: DEFAULT_API_BASE.to_owned(),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}

#[derive(Clone, Copy, Debug)]
enum Host {
    Deepseek,
    Tencent,
    Minimax,
}

// the capability seam answers by provider, so a wrong host tag changes the transformed request
#[derive(Clone, Copy, Default)]
struct Stub {
    effort_provider: Option<&'static str>,
}

impl ThinkingCapabilities for Stub {
    fn supports(
        &mut self,
        _model: &str,
        _key: &str,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn map_effort<'seam>(
        &'seam mut self,
        _model: &'seam str,
        _effort: &'seam str,
        _provider: &'seam str,
    ) -> impl Future<Output = Result<Option<Thinking>, ReasoningEffortError>> + Send + 'seam {
        ready(Ok(None))
    }

    fn validate_effort(
        &mut self,
        _model: &str,
        _effort: Option<&str>,
        _provider: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn supports_effort_param(
        &mut self,
        _model: &str,
        provider: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.effort_provider == Some(provider))
    }

    fn drop_disabled_thinking(
        &mut self,
        _model: &str,
        _params: &mut OptionalParams,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn translate_legacy_thinking<'seam>(
        &'seam mut self,
        _request: &'seam mut LegacyShapes<'seam>,
    ) -> impl Future<Output = Result<(), ThinkingError>> + Send + 'seam {
        ready(Ok(()))
    }
}

fn user_message() -> Json {
    let mut fields = Object::new();
    fields.insert("role".to_owned(), Json::Text("user".to_owned()));
    fields.insert("content".to_owned(), Json::Text("hi".to_owned()));
    Json::Object(fields)
}

fn custom_tool(name: &str, keeps_type: bool) -> Json {
    let mut fields = Object::new();
    if keeps_type {
        fields.insert("type".to_owned(), Json::Text("custom".to_owned()));
    }
    fields.insert("name".to_owned(), Json::Text(name.to_owned()));
    Json::Object(fields)
}

fn typed_tool(kind: &str) -> Json {
    let mut fields = Object::new();
    fields.insert("type".to_owned(), Json::Text(kind.to_owned()));
    Json::Object(fields)
}

fn transform_parameters(tools: Json) -> Object {
    let mut parameters = Object::new();
    parameters.insert("max_tokens".to_owned(), Json::Number(Number::from(128_i64)));
    parameters.insert("tools".to_owned(), tools);
    parameters
}

#[tokio::test]
async fn deepseek_strips_only_custom_tool_types() {
    let tools = Json::List(vec![
        custom_tool("bash", true),
        typed_tool("web_search_20250305"),
        Json::Text("passthrough".to_owned()),
    ]);
    let request = HostRequest {
        model: "deepseek-chat",
        messages: vec![user_message()],
        optional_params: transform_parameters(tools),
    };
    let mut stub = Stub::default();
    let body = deepseek::transform_request(&mut stub, request)
        .await
        .expect("the request transforms");
    let expected = Json::List(vec![
        custom_tool("bash", false),
        typed_tool("web_search_20250305"),
        Json::Text("passthrough".to_owned()),
    ]);
    assert_eq!(body.get("tools"), Some(&expected));
}

#[tokio::test]
async fn deepseek_leaves_a_non_list_tools_value_alone() {
    let request = HostRequest {
        model: "deepseek-chat",
        messages: vec![user_message()],
        optional_params: transform_parameters(Json::Text("not-a-list".to_owned())),
    };
    let mut stub = Stub::default();
    let body = deepseek::transform_request(&mut stub, request)
        .await
        .expect("the request transforms");
    assert_eq!(
        body.get("tools"),
        Some(&Json::Text("not-a-list".to_owned()))
    );
}

#[tokio::test]
async fn other_hosts_keep_custom_tool_types() {
    let tools = Json::List(vec![custom_tool("bash", true)]);
    let mut stub = Stub::default();
    let tencent_request = HostRequest {
        model: "hunyuan-t1",
        messages: vec![user_message()],
        optional_params: transform_parameters(tools.clone()),
    };
    let body = tencent::transform_request(&mut stub, tencent_request)
        .await
        .expect("the request transforms");
    assert_eq!(body.get("tools"), Some(&tools));

    let minimax_request = HostRequest {
        model: "MiniMax-M2.1",
        messages: vec![user_message()],
        optional_params: transform_parameters(tools.clone()),
    };
    let minimax_body = minimax::transform_request(&mut stub, minimax_request)
        .await
        .expect("the request transforms");
    assert_eq!(minimax_body.get("tools"), Some(&tools));
}

async fn transform_for_host<Capabilities>(
    host: Host,
    capabilities: &mut Capabilities,
    request: HostRequest<'_>,
) -> Result<Object, HostError>
where
    Capabilities: ThinkingCapabilities + Send,
{
    match host {
        Host::Deepseek => deepseek::transform_request(capabilities, request).await,
        Host::Tencent => tencent::transform_request(capabilities, request).await,
        Host::Minimax => minimax::transform_request(capabilities, request).await,
    }
}

async fn transform_with_system<Capabilities>(
    capabilities: &mut Capabilities,
    host: Host,
    system: Json,
) -> Object
where
    Capabilities: ThinkingCapabilities + Send,
{
    let mut parameters = Object::new();
    parameters.insert("max_tokens".to_owned(), Json::Number(Number::from(128_i64)));
    parameters.insert("system".to_owned(), system);
    transform_for_host(
        host,
        capabilities,
        HostRequest {
            model: "native-messages-model",
            messages: vec![user_message()],
            optional_params: parameters,
        },
    )
    .await
    .expect("the request transforms")
}

fn effort_parameters() -> Object {
    let mut output_config = Object::new();
    output_config.insert("effort".to_owned(), Json::Text("high".to_owned()));
    let mut parameters = Object::new();
    parameters.insert("max_tokens".to_owned(), Json::Number(Number::from(128_i64)));
    parameters.insert("output_config".to_owned(), Json::Object(output_config));
    parameters
}

#[rstest]
#[case::deepseek(Host::Deepseek, "deepseek")]
#[case::tencent(Host::Tencent, "tencent")]
#[case::minimax(Host::Minimax, "minimax")]
#[tokio::test]
async fn each_host_passes_its_provider_tag_to_the_thinking_rules(
    #[case] host: Host,
    #[case] provider: &'static str,
) {
    let kept = transform_for_host(
        host,
        &mut Stub {
            effort_provider: Some(provider),
        },
        HostRequest {
            model: "native-messages-model",
            messages: vec![user_message()],
            optional_params: effort_parameters(),
        },
    )
    .await
    .expect("the request transforms");
    assert!(
        kept.contains_key("output_config"),
        "{provider} keeps the native effort under its own tag"
    );

    let dropped = transform_for_host(
        host,
        &mut Stub {
            effort_provider: Some("some-other-provider"),
        },
        HostRequest {
            model: "native-messages-model",
            messages: vec![user_message()],
            optional_params: effort_parameters(),
        },
    )
    .await
    .expect("the request transforms");
    assert!(
        !dropped.contains_key("output_config"),
        "{provider} answers the capability seam with its own tag"
    );
}

#[tokio::test]
async fn all_hosts_strip_billing_system_metadata() {
    let system = Json::Text("x-anthropic-billing-header: cc_version=1".to_owned());
    let mut stub = Stub::default();
    let deepseek_body = transform_with_system(&mut stub, Host::Deepseek, system.clone()).await;
    let tencent_body = transform_with_system(&mut stub, Host::Tencent, system.clone()).await;
    let minimax_body = transform_with_system(&mut stub, Host::Minimax, system).await;
    assert_eq!(
        deepseek_body.get("system"),
        None,
        "deepseek strips the billing system block"
    );
    assert_eq!(
        tencent_body.get("system"),
        None,
        "tencent strips the billing system block"
    );
    assert_eq!(
        minimax_body.get("system"),
        None,
        "minimax strips the billing system block"
    );
}
