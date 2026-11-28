use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json as LogJson;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::gateway_provider_dispatch::call::CompletionCall;
use crate::gateway_provider_dispatch::chat::{
    OPENROUTER_DEFAULT_API_BASE, OPENROUTER_PROVIDER, VERCEL_PROVIDER, dispatch,
};
use crate::gateway_provider_dispatch::context::{DispatchContext, DispatchSettings};
use crate::gateway_provider_dispatch::error::DispatchError;
use crate::gateway_provider_dispatch::handler::HandlerError;

use super::support::{Fixture, Recorder, logging, object, redactor, secret, secret_text};

fn run(
    tag: ChatConfigTag,
    fixture: &Fixture,
    handler: &Recorder,
) -> (Option<Result<String, DispatchError>>, LoggingState) {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let context = DispatchContext {
        acompletion: false,
        api_base: fixture.api_base.as_deref(),
        api_key: fixture.api_key.as_ref(),
        custom_llm_provider: &fixture.provider,
        class_config: &fixture.class_config,
        headers: fixture.headers.as_ref(),
        litellm_params: &fixture.litellm_params,
        messages: &fixture.messages,
        model: &fixture.model,
        optional_params: &fixture.optional_params,
        stream: fixture.stream,
        timeout: fixture.timeout,
        settings: DispatchSettings::from(&fixture.settings),
    };
    let outcome = {
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        dispatch(tag, &context, handler, &mut log)
    };
    (outcome, state)
}

fn succeeded(outcome: Option<Result<String, DispatchError>>) -> String {
    outcome
        .expect("the dispatch owns this provider")
        .expect("the arm answers")
}

fn only_call(handler: &Recorder) -> CompletionCall {
    let mut calls = handler.calls();
    assert_eq!(calls.len(), 1, "the arm calls the handler once");
    calls.remove(0)
}

fn arm_call(tag: ChatConfigTag, fixture: &Fixture) -> CompletionCall {
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(tag, fixture, &handler);
    succeeded(outcome);
    only_call(&handler)
}

fn openrouter(fixture: &Fixture) -> CompletionCall {
    arm_call(ChatConfigTag::Openrouter, fixture)
}

fn openrouter_key(fixture: &Fixture) -> Option<String> {
    secret_text(openrouter(fixture).api_key.as_ref())
}

fn vercel(fixture: &Fixture) -> CompletionCall {
    arm_call(ChatConfigTag::VercelAiGateway, fixture)
}

fn headers(call: &CompletionCall) -> BTreeMap<String, String> {
    call.headers.clone().expect("the arm sets the headers")
}

#[rstest]
#[case::anthropic(ChatConfigTag::Anthropic)]
#[case::groq(ChatConfigTag::GroqChat)]
#[case::cloudflare(ChatConfigTag::CloudflareChat)]
#[case::fireworks(ChatConfigTag::FireworksAi)]
#[case::together(ChatConfigTag::TogetherAiChat)]
#[case::azure(ChatConfigTag::AzureOpenAi)]
fn dispatch_leaves_other_providers_alone(#[case] tag: ChatConfigTag) {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(tag, &fixture, &handler);
    assert!(outcome.is_none(), "{tag:?} belongs to another arm");
    assert!(handler.calls().is_empty());
}

#[rstest]
#[case::openrouter(ChatConfigTag::Openrouter, OPENROUTER_PROVIDER, "reply")]
#[case::vercel(ChatConfigTag::VercelAiGateway, VERCEL_PROVIDER, "reply")]
fn dispatch_runs_both_gateway_arms(
    #[case] tag: ChatConfigTag,
    #[case] provider: &str,
    #[case] reply: &str,
) {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(tag, &fixture, &handler);
    assert_eq!(succeeded(outcome), reply);
    assert_eq!(only_call(&handler).custom_llm_provider, provider);
}

#[test]
fn openrouter_base_falls_from_the_passed_value_to_the_global_then_the_environment_then_the_default()
{
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("http://passed"));
    fixture.settings.api_base = Some(String::from("http://global"));
    fixture.settings.environment.openrouter_api_base = Some(String::from("http://env"));
    assert_eq!(
        openrouter(&fixture).api_base.as_deref(),
        Some("http://passed")
    );

    fixture.api_base = None;
    assert_eq!(
        openrouter(&fixture).api_base.as_deref(),
        Some("http://global")
    );

    fixture.settings.api_base = Some(String::new());
    assert_eq!(openrouter(&fixture).api_base.as_deref(), Some("http://env"));

    fixture.settings.environment.openrouter_api_base = None;
    assert_eq!(
        openrouter(&fixture).api_base.as_deref(),
        Some(OPENROUTER_DEFAULT_API_BASE)
    );
}

#[test]
fn vercel_base_falls_from_the_passed_value_to_the_global_then_the_environment_then_the_default() {
    let mut fixture = Fixture::new();
    fixture.provider = String::from(VERCEL_PROVIDER);
    fixture.api_base = Some(String::from("http://passed"));
    fixture.settings.api_base = Some(String::from("http://global"));
    fixture.settings.environment.vercel_ai_gateway_api_base = Some(String::from("http://env"));
    assert_eq!(vercel(&fixture).api_base.as_deref(), Some("http://passed"));

    fixture.api_base = Some(String::new());
    assert_eq!(vercel(&fixture).api_base.as_deref(), Some("http://global"));

    fixture.settings.api_base = None;
    assert_eq!(vercel(&fixture).api_base.as_deref(), Some("http://env"));

    fixture.settings.environment.vercel_ai_gateway_api_base = None;
    assert_eq!(
        vercel(&fixture).api_base.as_deref(),
        Some("https://ai-gateway.vercel.sh/v1")
    );
}

#[test]
fn openrouter_key_follows_the_passed_global_openrouter_and_two_environment_values() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(secret("sk-passed"));
    fixture.settings.api_key = Some(secret("sk-global"));
    fixture.settings.openrouter_key = Some(secret("sk-openrouter"));
    fixture.settings.environment.openrouter_api_key = Some(secret("sk-openrouter-env"));
    fixture.settings.environment.or_api_key = Some(secret("sk-or"));
    assert_eq!(openrouter_key(&fixture), Some(String::from("sk-passed")));

    fixture.api_key = Some(secret(""));
    assert_eq!(
        openrouter_key(&fixture),
        Some(String::from("sk-global")),
        "an empty passed key falls through"
    );

    fixture.settings.api_key = None;
    assert_eq!(
        openrouter_key(&fixture),
        Some(String::from("sk-openrouter"))
    );

    fixture.settings.openrouter_key = Some(secret(""));
    assert_eq!(
        openrouter_key(&fixture),
        Some(String::from("sk-openrouter-env"))
    );

    fixture.settings.environment.openrouter_api_key = None;
    assert_eq!(openrouter_key(&fixture), Some(String::from("sk-or")));

    fixture.settings.environment.or_api_key = None;
    assert_eq!(openrouter_key(&fixture), None);
}

#[test]
fn vercel_key_stops_at_the_gateway_key_and_never_reads_the_oidc_token() {
    let mut fixture = Fixture::new();
    fixture.provider = String::from(VERCEL_PROVIDER);
    fixture.settings.environment.vercel_oidc_token = Some(secret("oidc"));
    assert_eq!(
        secret_text(vercel(&fixture).api_key.as_ref()),
        None,
        "the chat arm stops before VERCEL_OIDC_TOKEN"
    );

    fixture.settings.environment.vercel_ai_gateway_api_key = Some(secret("sk-vercel"));
    assert_eq!(
        secret_text(vercel(&fixture).api_key.as_ref()),
        Some(String::from("sk-vercel"))
    );

    fixture.settings.api_key = Some(secret("sk-global"));
    assert_eq!(
        secret_text(vercel(&fixture).api_key.as_ref()),
        Some(String::from("sk-global")),
        "the arm's own chain reads litellm.api_key before the environment key; \
         get_llm_provider's pre-resolved key wins over both in a public call"
    );

    fixture.settings.environment.vercel_ai_gateway_api_key = Some(secret(""));
    assert_eq!(
        secret_text(vercel(&fixture).api_key.as_ref()),
        Some(String::from("sk-global"))
    );
}

#[test]
fn the_vercel_chat_arm_never_falls_back_to_an_openai_key() {
    let lookup: BTreeMap<String, String> =
        BTreeMap::from([(String::from("OPENAI_API_KEY"), String::from("sk-openai"))]);
    let mut fixture = Fixture::new();
    fixture.provider = String::from(VERCEL_PROVIDER);
    fixture.settings = ConfigLoader { lookup }
        .load("")
        .expect("the fixture config loads")
        .sections
        .gateway_provider_keys;
    assert_eq!(
        secret_text(vercel(&fixture).api_key.as_ref()),
        None,
        "an OpenAI key is never sent to the Vercel gateway"
    );
}

#[test]
fn openrouter_headers_start_with_the_referer_defaults_in_their_exact_case() {
    let fixture = Fixture::new();
    assert_eq!(
        headers(&openrouter(&fixture)),
        BTreeMap::from([
            (
                String::from("HTTP-Referer"),
                String::from("https://litellm.ai")
            ),
            (String::from("X-Title"), String::from("liteLLM")),
        ])
    );
}

#[test]
fn openrouter_headers_read_the_site_and_app_from_the_environment() {
    let mut fixture = Fixture::new();
    fixture.settings.environment.or_site_url = Some(String::from("https://or.example"));
    fixture.settings.environment.or_app_name = Some(String::from("or-app"));
    assert_eq!(
        headers(&openrouter(&fixture)),
        BTreeMap::from([
            (
                String::from("HTTP-Referer"),
                String::from("https://or.example")
            ),
            (String::from("X-Title"), String::from("or-app")),
        ])
    );
}

#[test]
fn vercel_headers_use_the_request_or_the_global_map_and_never_a_referer_pair() {
    let mut fixture = Fixture::new();
    fixture.settings.environment.vercel_site_url = Some(String::from("https://vercel.example"));
    fixture.settings.environment.vercel_app_name = Some(String::from("vercel-app"));
    assert_eq!(
        vercel(&fixture).headers,
        None,
        "the env site and app never become headers"
    );

    fixture.settings.headers = Some(BTreeMap::from([(
        String::from("x-global"),
        String::from("one"),
    )]));
    assert_eq!(
        headers(&vercel(&fixture)),
        BTreeMap::from([(String::from("x-global"), String::from("one"))])
    );

    fixture.headers = Some(BTreeMap::from([(
        String::from("x-scope"),
        String::from("tenant"),
    )]));
    assert_eq!(
        headers(&vercel(&fixture)),
        BTreeMap::from([(String::from("x-scope"), String::from("tenant"))]),
        "request headers hide the global ones"
    );
}

#[test]
fn request_headers_replace_the_referer_values_and_keep_the_rest() {
    let mut fixture = Fixture::new();
    fixture.headers = Some(BTreeMap::from([
        (
            String::from("HTTP-Referer"),
            String::from("https://caller.example"),
        ),
        (String::from("x-scope"), String::from("tenant")),
    ]));
    assert_eq!(
        headers(&openrouter(&fixture)),
        BTreeMap::from([
            (
                String::from("HTTP-Referer"),
                String::from("https://caller.example")
            ),
            (String::from("X-Title"), String::from("liteLLM")),
            (String::from("x-scope"), String::from("tenant")),
        ])
    );
}

#[test]
fn an_empty_request_header_map_keeps_the_referer_defaults() {
    let mut fixture = Fixture::new();
    fixture.headers = Some(BTreeMap::new());
    fixture.settings.headers = Some(BTreeMap::from([(
        String::from("x-global"),
        String::from("one"),
    )]));
    assert_eq!(
        headers(&openrouter(&fixture)),
        BTreeMap::from([
            (
                String::from("HTTP-Referer"),
                String::from("https://litellm.ai")
            ),
            (String::from("X-Title"), String::from("liteLLM")),
            (String::from("x-global"), String::from("one")),
        ])
    );
}

#[test]
fn global_headers_are_laid_on_only_when_the_request_has_none() {
    let mut fixture = Fixture::new();
    fixture.settings.headers = Some(BTreeMap::from([
        (
            String::from("HTTP-Referer"),
            String::from("https://global.example"),
        ),
        (String::from("x-global"), String::from("one")),
    ]));
    assert_eq!(
        headers(&openrouter(&fixture)),
        BTreeMap::from([
            (
                String::from("HTTP-Referer"),
                String::from("https://global.example")
            ),
            (String::from("X-Title"), String::from("liteLLM")),
            (String::from("x-global"), String::from("one")),
        ])
    );

    fixture.headers = Some(BTreeMap::from([(
        String::from("x-scope"),
        String::from("tenant"),
    )]));
    assert_eq!(
        headers(&openrouter(&fixture)),
        BTreeMap::from([
            (
                String::from("HTTP-Referer"),
                String::from("https://litellm.ai")
            ),
            (String::from("X-Title"), String::from("liteLLM")),
            (String::from("x-scope"), String::from("tenant")),
        ]),
        "non-empty request headers hide the global ones"
    );
}

#[test]
fn the_class_config_adds_only_the_keys_the_request_lacks() {
    let mut fixture = Fixture::new();
    fixture.optional_params = object(r#"{"temperature": 0.5}"#);
    fixture.class_config = object(r#"{"temperature": 2, "stop": ["END"]}"#);
    let call = openrouter(&fixture);
    assert_eq!(
        call.optional_params,
        object(r#"{"temperature": 0.5, "stop": ["END"]}"#)
    );
}

#[test]
fn the_class_config_updates_an_existing_extra_body_key_by_key() {
    let mut fixture = Fixture::new();
    fixture.optional_params =
        object(r#"{"extra_body": {"transforms": ["caller"], "models": ["m"]}}"#);
    fixture.class_config =
        object(r#"{"extra_body": {"transforms": ["config"], "route": "fallback"}}"#);
    let call = openrouter(&fixture);
    assert_eq!(
        call.optional_params.get("extra_body"),
        Some(&Json::Object(object(
            r#"{"transforms": ["config"], "models": ["m"], "route": "fallback"}"#
        )))
    );
}

#[test]
fn the_class_config_sets_extra_body_when_the_request_has_none() {
    let mut fixture = Fixture::new();
    fixture.class_config = object(r#"{"extra_body": {"route": "fallback"}}"#);
    let call = openrouter(&fixture);
    assert_eq!(
        call.optional_params.get("extra_body"),
        Some(&Json::Object(object(r#"{"route": "fallback"}"#)))
    );
}

#[rstest]
#[case::request(
    r#"{"extra_body": "text"}"#,
    r#"{"extra_body": {"route": "fallback"}}"#
)]
#[case::config(
    r#"{"extra_body": {"route": "caller"}}"#,
    r#"{"extra_body": ["route"]}"#
)]
fn an_extra_body_that_is_not_an_object_is_refused(
    #[case] optional_params: &str,
    #[case] class_config: &str,
) {
    let mut fixture = Fixture::new();
    fixture.optional_params =
        serde_json::from_str(optional_params).expect("the request case parses");
    fixture.class_config = serde_json::from_str(class_config).expect("the config case parses");
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Openrouter, &fixture, &handler);
    let error = outcome
        .expect("the dispatch owns openrouter")
        .expect_err("the merge refuses a non-object extra_body");
    assert!(matches!(
        error,
        DispatchError::ExtraBody {
            provider: OPENROUTER_PROVIDER
        }
    ));
    assert!(
        handler.calls().is_empty(),
        "the arm stops before the handler"
    );
}

#[test]
fn the_vercel_arm_ignores_the_class_config() {
    let mut fixture = Fixture::new();
    fixture.provider = String::from(VERCEL_PROVIDER);
    fixture.optional_params = object(r#"{"temperature": 1}"#);
    fixture.class_config = object(r#"{"temperature": 2, "top_p": 1, "extra_body": 1}"#);
    let call = vercel(&fixture);
    assert_eq!(call.optional_params, object(r#"{"temperature": 1}"#));
}

#[test]
fn the_handler_failure_comes_back_with_its_status_and_no_post_log() {
    let fixture = Fixture::new();
    let handler = Recorder::failing(503);
    let (outcome, state) = run(ChatConfigTag::Openrouter, &fixture, &handler);
    let error = outcome
        .expect("the dispatch owns openrouter")
        .expect_err("the handler fails");
    assert_eq!(error.status(), Some(503));
    assert!(matches!(
        error,
        DispatchError::Handler(HandlerError::Upstream { status_code: 503 })
    ));
    assert!(state.model_call_details.get("original_response").is_none());
}

#[test]
fn a_successful_call_logs_post_call_with_the_messages() {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let (outcome, state) = run(ChatConfigTag::VercelAiGateway, &fixture, &handler);
    succeeded(outcome);
    let details = &state.model_call_details;
    assert_eq!(details.get("original_response"), Some(&LogJson::Null));
    assert_eq!(
        details.get("log_event_type"),
        Some(&LogJson::Text(String::from("post_api_call")))
    );
    let Some(LogJson::List(input)) = details.get("input") else {
        panic!("the post_call log holds the message list");
    };
    assert_eq!(input.len(), 1);
}

#[test]
fn both_arms_hand_the_request_fields_through_unchanged() {
    let mut fixture = Fixture::new();
    fixture.model = String::from("anthropic/claude-3.5-sonnet");
    fixture.stream = Some(true);
    fixture.timeout = Some(Duration::from_secs(30));
    fixture.litellm_params = object(r#"{"custom": "value"}"#);
    fixture.optional_params = object(r#"{"top_p": 1}"#);
    let call = openrouter(&fixture);
    assert_eq!(call.model, "anthropic/claude-3.5-sonnet");
    assert_eq!(call.stream, Some(true));
    assert_eq!(call.timeout, Some(Duration::from_secs(30)));
    assert_eq!(call.litellm_params, object(r#"{"custom": "value"}"#));
    assert_eq!(call.optional_params, object(r#"{"top_p": 1}"#));
    assert_eq!(call.messages.len(), 1);
    assert!(!call.acompletion);
}

#[test]
fn the_arm_forces_its_own_provider_string() {
    let mut fixture = Fixture::new();
    fixture.provider = String::from("something_else");
    assert_eq!(
        openrouter(&fixture).custom_llm_provider,
        OPENROUTER_PROVIDER
    );
    assert_eq!(vercel(&fixture).custom_llm_provider, VERCEL_PROVIDER);
}
