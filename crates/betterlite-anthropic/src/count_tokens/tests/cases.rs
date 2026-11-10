#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::anthropic_count_tokens::DEFAULT_BETA_VERSION;
use betterlite_config::sections::anthropic_credentials::Settings as CredentialSettings;
use betterlite_config::types::LiteLlmParams;
use betterlite_models::decisions::Json;
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use rstest::rstest;
use tracing::subscriber::set_default;

use crate::count_tokens::counter::{
    AnthropicTokenCounter, CountTokensCall, TokenCountResponse, TokenCounter as _,
};
use crate::count_tokens::error::{ApiError, CountTokensError};
use crate::count_tokens::handler::AnthropicCountTokensHandler;
use crate::count_tokens::headers::{DEFAULT_COUNT_TOKENS_ENDPOINT, get_required_headers};
use crate::count_tokens::model_info::AnthropicModelInfo;
use crate::count_tokens::validate::validate_request;
use crate::count_tokens::wire::{
    CountResponse, CountTokensBody, CountTokensRequest, decode_count_response, transform_request,
};
use crate::credentials::{ANTHROPIC_VERSION, AnthropicSettings};

use super::{
    Logs, MODEL, REQUEST_MODEL, Script, Sent, b198_request_timeout, deployment, env, handler,
    header, json, secret, subscriber, user_messages,
};

const SUCCESS: &[u8] = br#"{"input_tokens":5}"#;
const ERROR_BODY: &[u8] =
    br#"{"type":"error","error":{"type":"invalid_request_error","message":"bad"}}"#;
const ERROR_BODY_TEXT: &str =
    r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad"}}"#;

#[rstest]
#[case::count_endpoint(0)]
#[case::messages_system_tools(1)]
#[case::required_beta_header(2)]
#[case::model_validation(3)]
#[case::native_count_success(4)]
#[case::count_selection(5)]
#[case::token_counter_factory(6)]
#[case::count_api_error_result(7)]
#[tokio::test]
async fn count_tokens_cases(#[case] case: u8) {
    match case {
        0 => count_endpoint().await,
        1 => messages_system_tools(),
        2 => required_beta_header(),
        3 => model_validation(),
        4 => native_count_success().await,
        5 => count_selection().await,
        6 => token_counter_factory().await,
        _ => count_api_error_result().await,
    }
}

fn body_for<'call>(
    messages: &'call Json,
    options: &'call IndexMap<String, Json>,
) -> CountTokensBody<'call> {
    CountTokensBody {
        model: MODEL,
        messages,
        tools: None,
        system: None,
        optional_params: options,
    }
}

async fn handled(
    api_base: Option<&str>,
    timeout: Option<Duration>,
    request_timeout: Duration,
    body: CountTokensBody<'_>,
) -> (Sent, Result<CountResponse, ApiError>) {
    let local_handler = AnthropicCountTokensHandler {
        transport: Script::replying(200, SUCCESS),
        request_timeout,
        beta_version: DEFAULT_BETA_VERSION.to_owned(),
    };
    let key = secret("sk-case-endpoint");
    let request = CountTokensRequest {
        body,
        api_key: &key,
        api_base,
        timeout,
    };
    let outcome = local_handler.handle_count_tokens_request(&request).await;
    let recorded = local_handler.transport.request();
    (recorded, outcome)
}

async fn count_endpoint() {
    default_endpoint().await;
    empty_api_base().await;
    verbatim_endpoint().await;
    explicit_timeouts().await;
    configured_timeout().await;
}

async fn default_endpoint() {
    let messages = user_messages();
    let (sent, outcome) = handled(
        None,
        None,
        b198_request_timeout(),
        body_for(&messages, &IndexMap::new()),
    )
    .await;
    outcome.unwrap();
    assert_eq!(b198_request_timeout(), Duration::from_mins(100));
    assert_eq!(sent.method, Method::POST);
    assert_eq!(sent.url, DEFAULT_COUNT_TOKENS_ENDPOINT);
    assert_eq!(sent.deadline, Duration::from_mins(100));
}

async fn empty_api_base() {
    let messages = user_messages();
    let (sent, outcome) = handled(
        Some(""),
        None,
        b198_request_timeout(),
        body_for(&messages, &IndexMap::new()),
    )
    .await;
    outcome.unwrap();
    assert_eq!(sent.url, DEFAULT_COUNT_TOKENS_ENDPOINT);
}

async fn verbatim_endpoint() {
    let messages = user_messages();
    let queried_endpoint = "https://proxy.example/v1/messages/count_tokens?beta=1";
    let (queried, queried_outcome) = handled(
        Some(queried_endpoint),
        None,
        b198_request_timeout(),
        body_for(&messages, &IndexMap::new()),
    )
    .await;
    queried_outcome.unwrap();
    assert_eq!(queried.url, queried_endpoint);
    let (custom, custom_outcome) = handled(
        Some("http://custom.example"),
        None,
        b198_request_timeout(),
        body_for(&messages, &IndexMap::new()),
    )
    .await;
    custom_outcome.unwrap();
    assert_eq!(custom.url, "http://custom.example/");
}

async fn explicit_timeouts() {
    let messages = user_messages();
    let (zero, zero_outcome) = handled(
        None,
        Some(Duration::ZERO),
        b198_request_timeout(),
        body_for(&messages, &IndexMap::new()),
    )
    .await;
    zero_outcome.unwrap();
    assert_eq!(zero.deadline, Duration::ZERO);
    let (short, short_outcome) = handled(
        None,
        Some(Duration::from_secs(2)),
        Duration::from_secs(17),
        body_for(&messages, &IndexMap::new()),
    )
    .await;
    short_outcome.unwrap();
    assert_eq!(short.deadline, Duration::from_secs(2));
}

async fn configured_timeout() {
    let messages = user_messages();
    let (sent, outcome) = handled(
        None,
        None,
        Duration::from_secs(17),
        body_for(&messages, &IndexMap::new()),
    )
    .await;
    outcome.unwrap();
    assert_eq!(sent.deadline, Duration::from_secs(17));
}

fn messages_system_tools() {
    omitted_optionals();
    empty_optionals_are_kept();
    token_options_only();
    null_options_are_kept();
    empty_messages_with_system();
}

fn omitted_optionals() {
    let messages =
        json(r#"[{"role":"user","content":[{"type":"text","text":"hi","x":1}],"extra":true}]"#);
    let actual = transform_request(&body_for(&messages, &IndexMap::new()));
    let mut expected = IndexMap::new();
    expected.insert("model".to_owned(), Json::Text(MODEL.to_owned()));
    expected.insert("messages".to_owned(), messages);
    assert_eq!(actual, expected);
}

fn empty_optionals_are_kept() {
    let messages = user_messages();
    let system = Json::Text(String::new());
    let tools = Json::List(Vec::new());
    let body = CountTokensBody {
        model: MODEL,
        messages: &messages,
        tools: Some(&tools),
        system: Some(&system),
        optional_params: &IndexMap::new(),
    };
    let actual = transform_request(&body);
    assert_eq!(actual.get("system"), Some(&Json::Text(String::new())));
    assert_eq!(actual.get("tools"), Some(&Json::List(Vec::new())));
}

fn token_options_only() {
    let messages = user_messages();
    let options = json(
        r#"{"thinking":{"type":"enabled"},"tool_choice":{"type":"auto"},"output_config":{"format":"x"},"temperature":0.5,"max_tokens":5}"#,
    );
    let Json::Object(params) = options else {
        panic!("the optional params fixture is not an object");
    };
    let body = CountTokensBody {
        model: MODEL,
        messages: &messages,
        tools: None,
        system: None,
        optional_params: &params,
    };
    let actual = transform_request(&body);
    assert_eq!(actual.get("thinking"), Some(&json(r#"{"type":"enabled"}"#)));
    assert_eq!(actual.get("tool_choice"), Some(&json(r#"{"type":"auto"}"#)));
    assert_eq!(
        actual.get("output_config"),
        Some(&json(r#"{"format":"x"}"#))
    );
    assert!(!actual.contains_key("temperature"));
    assert!(!actual.contains_key("max_tokens"));
}

fn null_options_are_kept() {
    let messages = user_messages();
    let options = json(r#"{"thinking":null}"#);
    let Json::Object(params) = options else {
        panic!("the optional params fixture is not an object");
    };
    let body = CountTokensBody {
        model: MODEL,
        messages: &messages,
        tools: None,
        system: None,
        optional_params: &params,
    };
    assert_eq!(transform_request(&body).get("thinking"), Some(&Json::Null));
}

fn empty_messages_with_system() {
    let messages = Json::List(Vec::new());
    let system = user_messages();
    let body = CountTokensBody {
        model: MODEL,
        messages: &messages,
        tools: None,
        system: Some(&system),
        optional_params: &IndexMap::new(),
    };
    let actual = transform_request(&body);
    assert_eq!(actual.get("messages"), Some(&Json::List(Vec::new())));
    assert_eq!(actual.get("system"), Some(&system));
}

fn required_beta_header() {
    plain_key_headers();
    oauth_key_headers();
    prefixed_oauth_is_not_rewritten();
    configured_beta_is_sent();
}

fn map_header<'headers>(headers: &'headers HeaderMap, name: &str) -> Option<&'headers str> {
    let value = headers.get(name)?;
    value.to_str().ok()
}

fn plain_key_headers() {
    let headers = get_required_headers(&secret("sk-ant-api03-key"), DEFAULT_BETA_VERSION).unwrap();
    assert_eq!(
        map_header(&headers, "content-type"),
        Some("application/json")
    );
    assert_eq!(map_header(&headers, "x-api-key"), Some("sk-ant-api03-key"));
    assert_eq!(
        map_header(&headers, "anthropic-version"),
        Some("2023-06-01")
    );
    assert_eq!(
        map_header(&headers, "anthropic-beta"),
        Some(DEFAULT_BETA_VERSION)
    );
    assert_eq!(map_header(&headers, "authorization"), None);
    let empty = get_required_headers(&secret(""), DEFAULT_BETA_VERSION).unwrap();
    assert_eq!(map_header(&empty, "x-api-key"), Some(""));
}

fn oauth_key_headers() {
    let headers =
        get_required_headers(&secret("sk-ant-oat01-token"), DEFAULT_BETA_VERSION).unwrap();
    assert_eq!(map_header(&headers, "x-api-key"), None);
    assert_eq!(
        map_header(&headers, "authorization"),
        Some("Bearer sk-ant-oat01-token")
    );
    assert_eq!(
        map_header(&headers, "anthropic-beta"),
        Some("oauth-2025-04-20,token-counting-2024-11-01")
    );
    assert_eq!(
        map_header(&headers, "anthropic-dangerous-direct-browser-access"),
        Some("true")
    );
}

fn prefixed_oauth_is_not_rewritten() {
    let headers =
        get_required_headers(&secret("Bearer sk-ant-oat01-token"), DEFAULT_BETA_VERSION).unwrap();
    assert_eq!(
        map_header(&headers, "x-api-key"),
        Some("Bearer sk-ant-oat01-token")
    );
    assert_eq!(map_header(&headers, "authorization"), None);
}

fn configured_beta_is_sent() {
    let custom = "token-counting-2099-01-01";
    let plain = get_required_headers(&secret("sk-ant-api03-key"), custom).unwrap();
    assert_eq!(map_header(&plain, "anthropic-beta"), Some(custom));
    let oauth = get_required_headers(&secret("sk-ant-oat01-token"), custom).unwrap();
    assert_eq!(
        map_header(&oauth, "anthropic-beta"),
        Some("oauth-2025-04-20,token-counting-2099-01-01")
    );
}

fn validated(
    model: &str,
    messages: &Json,
    system: Option<&Json>,
    tools: Option<&Json>,
) -> Result<(), CountTokensError> {
    validate_request(&CountTokensBody {
        model,
        messages,
        tools,
        system,
        optional_params: &IndexMap::new(),
    })
}

fn model_validation() {
    validation_requires_model();
    validation_requires_messages();
    validation_requires_a_list();
    validation_checks_each_message();
    validation_accepts_valid_input();
}

fn validation_requires_model() {
    let messages = json(r#"[{"role":"user","content":"hi"}]"#);
    assert_eq!(
        validated("", &messages, None, None)
            .unwrap_err()
            .to_string(),
        "model parameter is required"
    );
    let wrong_shape = Json::Text("x".to_owned());
    assert_eq!(
        validated("", &wrong_shape, None, None)
            .unwrap_err()
            .to_string(),
        "model parameter is required"
    );
    validated(" ", &messages, None, None).unwrap();
}

fn validation_requires_messages() {
    let empty = Json::List(Vec::new());
    assert_eq!(
        validated("m", &empty, None, None).unwrap_err().to_string(),
        "messages parameter is required"
    );
    let null_messages = Json::Null;
    assert_eq!(
        validated("m", &null_messages, None, None)
            .unwrap_err()
            .to_string(),
        "messages parameter is required"
    );
    for falsey in [json("false"), json("0"), json("0.0")] {
        assert_eq!(
            validated("m", &falsey, None, None).unwrap_err().to_string(),
            "messages parameter is required"
        );
    }
    let blank = Json::Text(String::new());
    let empty_tools = Json::List(Vec::new());
    assert_eq!(
        validated("m", &empty, Some(&blank), Some(&empty_tools))
            .unwrap_err()
            .to_string(),
        "messages parameter is required"
    );
}

fn validation_requires_a_list() {
    for scalar in [json("true"), json("1"), json("0.5")] {
        assert_eq!(
            validated("m", &scalar, None, None).unwrap_err().to_string(),
            "messages must be a list"
        );
    }
    let text = Json::Text("nope".to_owned());
    assert_eq!(
        validated("m", &text, None, None).unwrap_err().to_string(),
        "messages must be a list"
    );
    let object = json(r#"{"role":"user"}"#);
    assert_eq!(
        validated("m", &object, None, None).unwrap_err().to_string(),
        "messages must be a list"
    );
}

fn validation_checks_each_message() {
    let scalar = json("[1]");
    assert_eq!(
        validated("m", &scalar, None, None).unwrap_err().to_string(),
        "Message 0 must be a dictionary"
    );
    let missing_content = json(r#"[{"role":"user"}]"#);
    assert_eq!(
        validated("m", &missing_content, None, None)
            .unwrap_err()
            .to_string(),
        "Message 0 must have a 'content' field"
    );
    let missing_role = json(r#"[{"content":"hi"}]"#);
    assert_eq!(
        validated("m", &missing_role, None, None)
            .unwrap_err()
            .to_string(),
        "Message 0 must have a 'role' field"
    );
    let second_scalar = json(r#"[{"role":"user","content":"hi"},5]"#);
    assert_eq!(
        validated("m", &second_scalar, None, None)
            .unwrap_err()
            .to_string(),
        "Message 1 must be a dictionary"
    );
}

fn validation_accepts_valid_input() {
    let messages = json(r#"[{"role":"user","content":"hi"}]"#);
    validated("m", &messages, None, None).unwrap();
    let nulls = json(r#"[{"role":null,"content":null}]"#);
    validated("m", &nulls, None, None).unwrap();
    let empty = Json::List(Vec::new());
    let system = Json::Text("sys".to_owned());
    validated("m", &empty, Some(&system), None).unwrap();
    let tools = json(r#"[{"name":"t"}]"#);
    validated("m", &empty, None, Some(&tools)).unwrap();
}

fn counted_call<'call>(
    model: &'call str,
    messages: &'call Json,
    deployment: Option<&'call LiteLlmParams>,
) -> CountTokensCall<'call> {
    CountTokensCall {
        model_to_use: model,
        messages: Some(messages),
        contents: None,
        deployment,
        request_model: REQUEST_MODEL,
        tools: None,
        system: None,
    }
}

async fn native_count_success() {
    native_success_maps_fields().await;
    native_success_defaults_input_tokens().await;
}

async fn native_success_maps_fields() {
    let messages = user_messages();
    let params = deployment(Some("sk-deployment"));
    let counter = AnthropicTokenCounter {
        handler: handler(Script::replying(
            200,
            br#"{"input_tokens":42,"extra":{"a":1}}"#,
        )),
        environment_api_key: None,
    };
    let outcome = counter
        .count_tokens(&counted_call(MODEL, &messages, Some(&params)))
        .await
        .unwrap();
    assert_eq!(outcome.total_tokens, 42);
    assert_eq!(outcome.request_model, REQUEST_MODEL);
    assert_eq!(outcome.model_used, MODEL);
    assert_eq!(outcome.tokenizer_type, "anthropic_api");
    assert!(!outcome.error);
    assert_eq!(outcome.error_message, None);
    assert_eq!(outcome.status_code, None);
    let original = outcome.original_response.unwrap();
    assert_eq!(
        original.get("input_tokens"),
        Some(&Json::Number(serde_json::Number::from(42_u8)))
    );
    assert_eq!(original.get("extra"), Some(&json(r#"{"a":1}"#)));
    let sent = counter.handler.transport.request();
    assert_eq!(
        serde_json::from_slice::<Json>(&sent.body).unwrap(),
        json(
            r#"{"model":"claude-3-5-sonnet-20241022","messages":[{"role":"user","content":[{"type":"text","text":"hi"}],"extra":true}]}"#
        )
    );
}

async fn native_success_defaults_input_tokens() {
    let messages = user_messages();
    let params = deployment(Some("sk-deployment"));
    let counter = AnthropicTokenCounter {
        handler: handler(Script::replying(200, b"{}")),
        environment_api_key: None,
    };
    let outcome = counter
        .count_tokens(&counted_call(MODEL, &messages, Some(&params)))
        .await
        .unwrap();
    assert_eq!(outcome.total_tokens, 0);
    assert_eq!(outcome.original_response, Some(IndexMap::new()));
    decode_count_response(b"null").unwrap_err();
}

async fn count_selection() {
    selection_matches_the_provider();
    falsey_messages_skip_the_request().await;
    deployment_key_beats_the_environment().await;
    missing_key_warns_and_returns_none().await;
    deployment_connection_fields_are_not_forwarded().await;
    contents_are_ignored().await;
    tools_and_system_are_forwarded().await;
}

fn selection_matches_the_provider() {
    let counter = AnthropicTokenCounter {
        handler: handler(Script::replying(200, SUCCESS)),
        environment_api_key: None,
    };
    assert!(counter.should_use_token_counting_api(Some("anthropic")));
    assert!(!counter.should_use_token_counting_api(Some("azure_ai")));
    assert!(!counter.should_use_token_counting_api(Some("Anthropic")));
    assert!(!counter.should_use_token_counting_api(Some("")));
    assert!(!counter.should_use_token_counting_api(None));
}

async fn falsey_messages_skip_the_request() {
    let empty = Json::List(Vec::new());
    let system = Json::Text("sys".to_owned());
    let tools = json(r#"[{"name":"t"}]"#);
    let contents = json(r#"[{"type":"text","text":"ignored"}]"#);
    let params = deployment(Some("sk-deployment"));
    let counter = AnthropicTokenCounter {
        handler: handler(Script::replying(200, SUCCESS)),
        environment_api_key: None,
    };
    let empty_call = CountTokensCall {
        model_to_use: MODEL,
        messages: Some(&empty),
        contents: Some(&contents),
        deployment: Some(&params),
        request_model: REQUEST_MODEL,
        tools: Some(&tools),
        system: Some(&system),
    };
    assert!(counter.count_tokens(&empty_call).await.is_none());
    let absent_call = CountTokensCall {
        model_to_use: MODEL,
        messages: None,
        contents: Some(&contents),
        deployment: Some(&params),
        request_model: REQUEST_MODEL,
        tools: Some(&tools),
        system: Some(&system),
    };
    assert!(counter.count_tokens(&absent_call).await.is_none());
    assert!(counter.handler.transport.requests().is_empty());
}

async fn deployment_key_beats_the_environment() {
    let messages = user_messages();
    let params = deployment(Some("sk-deployment"));
    let empty_key = deployment(Some(""));
    let environment_counter = AnthropicTokenCounter {
        handler: handler(Script::replying(200, SUCCESS)),
        environment_api_key: Some(secret("sk-environment")),
    };
    environment_counter
        .count_tokens(&counted_call(MODEL, &messages, Some(&empty_key)))
        .await
        .unwrap();
    assert_eq!(
        header(
            &environment_counter.handler.transport.request(),
            "x-api-key"
        ),
        Some("sk-environment")
    );
    let deployment_counter = AnthropicTokenCounter {
        handler: handler(Script::replying(200, SUCCESS)),
        environment_api_key: Some(secret("sk-environment")),
    };
    deployment_counter
        .count_tokens(&counted_call(MODEL, &messages, Some(&params)))
        .await
        .unwrap();
    assert_eq!(
        header(&deployment_counter.handler.transport.request(), "x-api-key"),
        Some("sk-deployment")
    );
}

async fn missing_key_warns_and_returns_none() {
    let messages = user_messages();
    let resolved = credential_settings(&env(&[("ANTHROPIC_AUTH_TOKEN", "auth-token")]));
    assert!(resolved.api_key.is_none());
    let counter = AnthropicTokenCounter {
        handler: handler(Script::replying(200, SUCCESS)),
        environment_api_key: resolved.api_key,
    };
    let logs = Logs::default();
    let outcome = {
        let _guard = set_default(subscriber(logs.clone()));
        counter
            .count_tokens(&counted_call(MODEL, &messages, None))
            .await
    };
    assert!(outcome.is_none());
    assert!(counter.handler.transport.requests().is_empty());
    assert_eq!(
        logs.messages(),
        vec!["No Anthropic API key found for token counting".to_owned()]
    );
}

async fn deployment_connection_fields_are_not_forwarded() {
    let messages = user_messages();
    let mut params = deployment(Some("sk-deployment"));
    params.api_base = Some("http://deployment.example".to_owned());
    params.timeout = Some(7.0_f64);
    let counter = AnthropicTokenCounter {
        handler: handler(Script::replying(200, SUCCESS)),
        environment_api_key: None,
    };
    counter
        .count_tokens(&counted_call(MODEL, &messages, Some(&params)))
        .await
        .unwrap();
    let sent = counter.handler.transport.request();
    assert_eq!(sent.url, DEFAULT_COUNT_TOKENS_ENDPOINT);
    assert_eq!(sent.deadline, b198_request_timeout());
    assert_eq!(
        serde_json::from_slice::<Json>(&sent.body).unwrap(),
        json(
            r#"{"model":"claude-3-5-sonnet-20241022","messages":[{"role":"user","content":[{"type":"text","text":"hi"}],"extra":true}]}"#
        )
    );
}

async fn contents_are_ignored() {
    let messages = user_messages();
    let params = deployment(Some("sk-deployment"));
    let contents = json(r#"[{"type":"text","text":"ignored"}]"#);
    let counter = AnthropicTokenCounter {
        handler: handler(Script::replying_with(&[(200, SUCCESS), (200, SUCCESS)])),
        environment_api_key: None,
    };
    let with_contents = CountTokensCall {
        model_to_use: MODEL,
        messages: Some(&messages),
        contents: Some(&contents),
        deployment: Some(&params),
        request_model: REQUEST_MODEL,
        tools: None,
        system: None,
    };
    counter.count_tokens(&with_contents).await.unwrap();
    counter
        .count_tokens(&counted_call(MODEL, &messages, Some(&params)))
        .await
        .unwrap();
    let requests = counter.handler.transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body, requests[1].body);
}

async fn tools_and_system_are_forwarded() {
    let messages = user_messages();
    let params = deployment(Some("sk-deployment"));
    let tools = json(r#"[{"name":"t"}]"#);
    let system = Json::Text("sys".to_owned());
    let counter = AnthropicTokenCounter {
        handler: handler(Script::replying(200, SUCCESS)),
        environment_api_key: None,
    };
    let with_options = CountTokensCall {
        model_to_use: MODEL,
        messages: Some(&messages),
        contents: None,
        deployment: Some(&params),
        request_model: REQUEST_MODEL,
        tools: Some(&tools),
        system: Some(&system),
    };
    counter.count_tokens(&with_options).await.unwrap();
    let body = serde_json::from_slice::<Json>(&counter.handler.transport.request().body).unwrap();
    let Json::Object(fields) = body else {
        panic!("the CountTokens request body is not a JSON object");
    };
    assert_eq!(fields.get("tools"), Some(&tools));
    assert_eq!(fields.get("system"), Some(&system));
}

fn credential_settings(environment: &BTreeMap<String, String>) -> AnthropicSettings {
    let resolved = CredentialSettings::resolve_environment(environment).unwrap();
    AnthropicSettings {
        api_key: resolved.api_key,
        auth_token: resolved.auth_token,
        api_base: resolved.api_base,
        use_bearer_for_custom_base: resolved.use_bearer_for_custom_base,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}

async fn token_counter_factory() {
    factory_is_model_independent().await;
    factory_supplies_the_environment_key().await;
}

async fn factory_is_model_independent() {
    let messages = user_messages();
    let params = deployment(Some("sk-factory"));
    let settings = credential_settings(&env(&[("ANTHROPIC_API_KEY", "sk-factory-environment")]));
    let model_info = AnthropicModelInfo { settings };
    let counter = model_info.get_token_counter(handler(Script::replying_with(&[
        (200, SUCCESS),
        (200, SUCCESS),
        (200, SUCCESS),
    ])));
    let first = counter
        .count_tokens(&counted_call(MODEL, &messages, Some(&params)))
        .await
        .unwrap();
    let second = counter
        .count_tokens(&counted_call("gpt-4o", &messages, Some(&params)))
        .await
        .unwrap();
    let third = counter
        .count_tokens(&counted_call(
            "anthropic/claude-opus-4-1",
            &messages,
            Some(&params),
        ))
        .await
        .unwrap();
    assert_eq!(first.tokenizer_type, "anthropic_api");
    assert_eq!(second.tokenizer_type, "anthropic_api");
    assert_eq!(third.tokenizer_type, "anthropic_api");
    assert_eq!(first.model_used, MODEL);
    assert_eq!(second.model_used, "gpt-4o");
    assert_eq!(third.model_used, "anthropic/claude-opus-4-1");
    let requests = counter.handler.transport.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].url, DEFAULT_COUNT_TOKENS_ENDPOINT);
    assert_eq!(requests[1].url, DEFAULT_COUNT_TOKENS_ENDPOINT);
    assert_eq!(requests[2].url, DEFAULT_COUNT_TOKENS_ENDPOINT);
}

async fn factory_supplies_the_environment_key() {
    let messages = user_messages();
    let settings = credential_settings(&env(&[("ANTHROPIC_API_KEY", "sk-factory-environment")]));
    let model_info = AnthropicModelInfo { settings };
    let counter = model_info.get_token_counter(handler(Script::replying(200, SUCCESS)));
    counter
        .count_tokens(&counted_call(MODEL, &messages, None))
        .await
        .unwrap();
    assert_eq!(
        header(&counter.handler.transport.request(), "x-api-key"),
        Some("sk-factory-environment")
    );
}

type Counted = (Option<TokenCountResponse>, Vec<String>);

async fn counted(script: Script, params: Option<&LiteLlmParams>, messages: &Json) -> Counted {
    let counter = AnthropicTokenCounter {
        handler: handler(script),
        environment_api_key: None,
    };
    let logs = Logs::default();
    let outcome = {
        let _guard = set_default(subscriber(logs.clone()));
        counter
            .count_tokens(&counted_call(MODEL, messages, params))
            .await
    };
    (outcome, logs.messages())
}

async fn count_api_error_result() {
    non_200_status_and_text_are_kept().await;
    transport_failure_is_a_500_record().await;
    undecodable_body_is_a_500_record().await;
    non_integer_input_tokens_is_a_generic_error().await;
}

async fn non_200_status_and_text_are_kept() {
    let messages = user_messages();
    let params = deployment(Some("sk-error"));
    let (outcome, warnings) = counted(
        Script::replying(429, b"rate limited"),
        Some(&params),
        &messages,
    )
    .await;
    let failure = outcome.unwrap();
    assert!(failure.error);
    assert_eq!(failure.total_tokens, 0);
    assert_eq!(failure.status_code, Some(429));
    assert_eq!(failure.error_message.as_deref(), Some("rate limited"));
    assert_eq!(failure.original_response, None);
    assert_eq!(
        warnings,
        vec!["Anthropic CountTokens API error: status=429, message=rate limited".to_owned()]
    );
    let (bad_outcome, bad_warnings) =
        counted(Script::replying(400, ERROR_BODY), Some(&params), &messages).await;
    let bad_request = bad_outcome.unwrap();
    assert_eq!(bad_request.status_code, Some(400));
    assert_eq!(bad_request.error_message.as_deref(), Some(ERROR_BODY_TEXT));
    assert!(bad_warnings[0].starts_with("Anthropic CountTokens API error: status=400"));
}

async fn transport_failure_is_a_500_record() {
    let messages = user_messages();
    let params = deployment(Some("sk-error"));
    let (outcome, warnings) = counted(Script::failing(), Some(&params), &messages).await;
    let failure = outcome.unwrap();
    assert!(failure.error);
    assert_eq!(failure.status_code, Some(500));
    assert!(
        failure
            .error_message
            .as_deref()
            .is_some_and(|message| message.starts_with("CountTokens processing error: "))
    );
    assert!(warnings[0].starts_with("Anthropic CountTokens API error: status=500"));
}

async fn undecodable_body_is_a_500_record() {
    let messages = user_messages();
    let params = deployment(Some("sk-error"));
    let (outcome, _) = counted(Script::replying(200, b"not json"), Some(&params), &messages).await;
    assert_eq!(outcome.unwrap().status_code, Some(500));
    let (array_outcome, _) = counted(Script::replying(200, b"[]"), Some(&params), &messages).await;
    assert_eq!(array_outcome.unwrap().status_code, Some(500));
}

async fn non_integer_input_tokens_is_a_generic_error() {
    let messages = user_messages();
    let params = deployment(Some("sk-error"));
    let (outcome, warnings) = counted(
        Script::replying(200, br#"{"input_tokens":null}"#),
        Some(&params),
        &messages,
    )
    .await;
    let failure = outcome.unwrap();
    assert!(failure.error);
    assert_eq!(failure.total_tokens, 0);
    assert_eq!(failure.status_code, Some(500));
    assert_eq!(
        failure.error_message.as_deref(),
        Some("input_tokens is not an integer")
    );
    assert_eq!(
        warnings,
        vec!["Error calling Anthropic CountTokens API: input_tokens is not an integer".to_owned()]
    );
    let (string_outcome, _) = counted(
        Script::replying(200, br#"{"input_tokens":"7"}"#),
        Some(&params),
        &messages,
    )
    .await;
    assert_eq!(string_outcome.unwrap().status_code, Some(500));
    let (float_outcome, _) = counted(
        Script::replying(200, br#"{"input_tokens":3.0}"#),
        Some(&params),
        &messages,
    )
    .await;
    assert_eq!(float_outcome.unwrap().status_code, Some(500));
}
