use std::collections::BTreeMap;

use betterlite_config::sections::assistant_settings::{Config, Settings};
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::super::error::AssistantDispatchError;
use super::super::operation::AssistantOperation;
use super::*;

async fn a_configured_parameter_overlays_the_request(settings: &Settings) {
    let fixtures = fixture(settings);
    let original = settings.clone();
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[("api_key", text("request-key")), ("model", text("gpt-4o"))]),
        Route::default(),
        &[(200, ASSISTANT)],
    )
    .await;
    outcome.result.expect("the configured call succeeds");
    assert_eq!(
        outcome.seen[0].authorization.as_deref(),
        Some("Bearer config-key")
    );
    let Json::Object(fields) = &outcome.seen[0].body else {
        panic!("the provider body is not an object");
    };
    assert_eq!(fields.get("model"), Some(&text("gpt-4o-mini")));
    assert_eq!(*settings, original, "the configured map stays unchanged");
    let params = original
        .config
        .expect("configured")
        .litellm_params
        .expect("parameters");
    assert_eq!(params.get("api_key"), Some(&text("config-key")));
}

async fn an_explicit_provider_bypasses_the_config(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[
            ("custom_llm_provider", text("azure")),
            ("api_key", text("request-key")),
            ("api_base", text(MOCK_BASE)),
            ("api_version", text("2024-05-01-preview")),
            ("model", text("gpt-4o")),
        ]),
        Route::default(),
        &[(200, ASSISTANT)],
    )
    .await;
    outcome.result.expect("the explicit azure call succeeds");
    assert_eq!(outcome.seen[0].api_key.as_deref(), Some("request-key"));
    let Json::Object(fields) = &outcome.seen[0].body else {
        panic!("the provider body is not an object");
    };
    assert_eq!(fields.get("model"), Some(&text("gpt-4o")));
}

// any provider create_assistants does not support takes the same rejection, before any HTTP
async fn a_rejected_provider(settings: &Settings, provider: &str, mentioned: &str) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[
            ("custom_llm_provider", text(provider)),
            ("api_key", text("request-key")),
            ("model", text("gpt-4o")),
        ]),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::Failure { failure }) = outcome.result else {
        panic!("the unsupported provider did not surface as a failure");
    };
    assert_eq!(failure.class, "BadRequestError");
    assert_eq!(failure.status, Some(400));
    assert!(failure.message.contains(mentioned), "{failure:?}");
    assert!(outcome.seen.is_empty());
}

async fn configured_parameters_replace_the_path_keyword() {
    let settings = configured("openai", &[("assistant_id", text("asst_configured"))]);
    let fixtures = fixture(&settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::DeleteAssistant,
        &openai_data(),
        Route {
            thread_id: None,
            assistant_id: Some("asst_a"),
            client: None,
        },
        &[(200, DELETED)],
    )
    .await;
    outcome.result.expect("the configured id call succeeds");
    assert_eq!(
        outcome.seen[0].url,
        "https://mock.test/v1/assistants/asst_configured"
    );
}

#[tokio::test]
async fn assistant_config_overlays_request() {
    let settings = configured(
        "openai",
        &[
            ("api_key", text("config-key")),
            ("model", text("gpt-4o-mini")),
        ],
    );
    a_configured_parameter_overlays_the_request(&settings).await;
    an_explicit_provider_bypasses_the_config(&settings).await;
    a_rejected_provider(&settings, "", "doesn't support").await;
    a_rejected_provider(&settings, "anthropic", "doesn't support anthropic").await;
    configured_parameters_replace_the_path_keyword().await;
}

// a configured client or provider collides with the installed function's own named arguments
#[rstest]
#[case::client("client")]
#[case::provider("custom_llm_provider")]
#[tokio::test]
async fn a_configured_keyword_collides_with_the_installed_function(#[case] name: &str) {
    let colliding = configured("openai", &[(name, text("configured"))]);
    let fixtures = fixture(&colliding);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[("api_key", text("request-key")), ("model", text("gpt-4o"))]),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::TypeError { message }) = outcome.result else {
        panic!("the configured keyword did not collide");
    };
    assert_eq!(
        message,
        format!(
            "litellm.assistants.main.acreate_assistants() got multiple values for keyword argument '{name}'"
        )
    );
    assert!(outcome.seen.is_empty());
}

// the route passes the path id as a keyword beside **data, so a body copy of that key collides
#[tokio::test]
async fn a_body_key_collides_with_the_route_argument() {
    let settings = configured("openai", &[]);
    let fixtures = fixture(&settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::DeleteAssistant,
        &data(&[("assistant_id", text("asst_body"))]),
        Route {
            thread_id: None,
            assistant_id: Some("asst_a"),
            client: None,
        },
        &[],
    )
    .await;
    let Err(AssistantDispatchError::TypeError { message }) = outcome.result else {
        panic!("the body assistant id did not collide");
    };
    assert_eq!(
        message,
        "litellm.router.Router.factory_function.<locals>.async_wrapper() got multiple values for keyword argument 'assistant_id'"
    );
    assert!(outcome.seen.is_empty());
}

async fn no_configured_provider_raises_the_factory_exception() {
    let absent = Settings::default();
    let fixtures = fixture(&absent);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[("model", text("gpt-4o-mini")), ("api_key", text("sk-test"))]),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::Exception { message }) = outcome.result else {
        panic!("no provider did not raise the factory exception");
    };
    assert_eq!(
        message,
        "'custom_llm_provider' must be set. Either via:\n `Router(assistants_config={'custom_llm_provider': ..})` \nor\n `router.arun_thread(custom_llm_provider=..)`"
    );
    assert!(outcome.seen.is_empty());
}

// assistants_config reads both keys as plain dictionary entries, so an absent one is a KeyError
async fn a_missing_configured_key_raises_key_error(
    provider: Option<&str>,
    parameters: Option<BTreeMap<String, Json>>,
    expected: &str,
) {
    let settings = Settings {
        config: Some(Config {
            custom_llm_provider: provider.map(String::from),
            litellm_params: parameters,
        }),
    };
    let outcome = call(
        &settings,
        AssistantOperation::CreateAssistant,
        &data(&[("model", text("gpt-4o-mini"))]),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::KeyError { key }) = outcome.result else {
        panic!("the missing configured key did not raise");
    };
    assert_eq!(key, expected);
    assert!(outcome.seen.is_empty());
}

#[tokio::test]
async fn assistant_dispatch_requires_provider() {
    no_configured_provider_raises_the_factory_exception().await;
    a_missing_configured_key_raises_key_error(None, Some(BTreeMap::new()), "custom_llm_provider")
        .await;
    a_missing_configured_key_raises_key_error(Some("openai"), None, "litellm_params").await;
    let absent = Settings::default();
    a_rejected_provider(&absent, "anthropic", "'create_assistants'").await;
}
