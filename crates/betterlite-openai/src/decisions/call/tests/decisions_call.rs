use std::collections::BTreeMap;
use std::error::Error;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::{DecisionsJson, Extra, Json};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::credentials::LiteLlmGlobals;
use crate::decisions::call::error::PrepareError;
use crate::decisions::call::fields::{
    OpenAiDecisionRequestFields, RequestFields, RequestFormat, SystemOneRequestFields,
};
use crate::decisions::call::prepare::credentials;
use crate::decisions::call::provider::ProviderConfig;
use crate::decisions::call::reject_other_format;
use crate::decisions::call::{CredentialInput, Selection};
use crate::decisions::openai::config::Credential;
use crate::decisions::openai::request::wire::{InputWire, QuestionWire};
use crate::decisions::providers::DecisionsProvider;
use crate::decisions::transformation::requests::RequestConversionError;
use crate::decisions::transformation::wire::ProviderRequestWire;
use crate::openai_base_config::Headers;

use super::support::{
    CallBuilder, Fixture, azure_ai, environment, exposed, globals, good_systemone_fields, json,
    noul_question, openai_empty_name_fields, openai_image_fields, openai_predicate_fields,
    openai_wire, predicate_question, question_map, secret, systemone_single_choice_fields,
    systemone_wire,
};

#[test]
fn prepare_call_success_builds_the_call() {
    let fixture = Fixture::default();
    let key = secret("sk-explicit");
    let mut builder =
        CallBuilder::new("openai/gpt-4o-mini", openai_predicate_fields("Is it true?"));
    builder.api_key = Some(&key);
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(call.model, "gpt-4o-mini");
    assert_eq!(call.requested_model, "openai/gpt-4o-mini");
    assert_eq!(call.custom_llm_provider, "openai");
    assert_eq!(call.api_base, "https://api.openai.com");
    assert_eq!(exposed(call.api_key.as_ref()), Some("sk-explicit"));
    assert!(call.headers.is_empty());
    assert!(call.timeout.is_none());
    assert!(call.logging_obj.is_none());
    assert!(matches!(&call.request, RequestFormat::OpenAi(_)));
}

#[test]
fn prepare_call_success_transforms_the_wire_body() {
    let fixture = Fixture::default();
    let key = secret("sk-explicit");
    let mut builder =
        CallBuilder::new("openai/gpt-4o-mini", openai_predicate_fields("Is it true?"));
    builder.api_key = Some(&key);
    let call = fixture.prepare(builder, None).unwrap();
    let wire = openai_wire(&call.body);
    assert_eq!(wire.model, "gpt-4o-mini");
    match &wire.input {
        InputWire::Text(text) => assert_eq!(text, "Is it true?"),
        InputWire::Messages(_messages) => panic!("an OpenAI text input stays text"),
    }
    match &wire.questions[0] {
        QuestionWire::Predicate { name, instructions } => {
            assert_eq!(name.as_deref(), Some("first"));
            assert_eq!(instructions, "Is it true?");
        }
        other @ (QuestionWire::Choice { .. } | QuestionWire::Score { .. }) => {
            panic!("expected a predicate question, got {other:?}")
        }
    }
}

#[derive(Clone, Copy)]
struct ProviderCase {
    provider: DecisionsProvider,
    model: &'static str,
    api_base: Option<&'static str>,
    expected_base: &'static str,
    expected_canonical: &'static str,
    config_name: &'static str,
}

#[rstest]
#[case::openai(ProviderCase { provider: DecisionsProvider::Openai, model: "openai/gpt-4o-mini", api_base: None, expected_base: "https://api.openai.com", expected_canonical: "gpt-4o-mini", config_name: "Openai" })]
#[case::openrouter(ProviderCase { provider: DecisionsProvider::Openrouter, model: "openrouter/sonar", api_base: None, expected_base: "https://openrouter.ai/api", expected_canonical: "sonar", config_name: "Openrouter" })]
#[case::azure_ai(ProviderCase { provider: DecisionsProvider::AzureAi, model: "azure_ai/gpt-4o-mini", api_base: Some("https://res.services.ai.azure.com"), expected_base: "https://res.services.ai.azure.com", expected_canonical: "gpt-4o-mini", config_name: "AzureAi" })]
#[case::perplexity(ProviderCase { provider: DecisionsProvider::Perplexity, model: "perplexity/sonar", api_base: None, expected_base: "https://api.perplexity.ai", expected_canonical: "sonar", config_name: "Perplexity" })]
#[case::typesafe(ProviderCase { provider: DecisionsProvider::Typesafe, model: "typesafe/gpt-4o", api_base: Some("https://typesafe.test"), expected_base: "https://typesafe.test", expected_canonical: "gpt-4o", config_name: "Typesafe" })]
#[case::strands_decider(ProviderCase { provider: DecisionsProvider::StrandsDecider, model: "strands_decider/my-agent", api_base: Some("https://strands.test"), expected_base: "https://strands.test", expected_canonical: "my-agent", config_name: "StrandsDecider" })]
#[case::cloudflare(ProviderCase { provider: DecisionsProvider::Cloudflare, model: "cloudflare/llama-3", api_base: Some("https://api.cloudflare.test/client/v4/accounts/1/ai/run"), expected_base: "https://api.cloudflare.test/client/v4/accounts/1/ai/run", expected_canonical: "@cf/cloudflare/llama-3", config_name: "Cloudflare" })]
#[case::hosted_vllm(ProviderCase { provider: DecisionsProvider::HostedVllm, model: "hosted_vllm/my-model", api_base: Some("http://vllm.test:8000"), expected_base: "http://vllm.test:8000", expected_canonical: "my-model", config_name: "HostedVllm" })]
fn prepare_call_success_selects_each_provider_config(#[case] case: ProviderCase) {
    let fixture = Fixture::default();
    let key = secret("sk-provider");
    let mut builder = CallBuilder::new(case.model, good_systemone_fields("Is it green?"));
    builder.api_base = case.api_base;
    builder.api_key = Some(&key);
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(call.custom_llm_provider, case.provider.as_str());
    assert_eq!(call.model, case.expected_canonical);
    assert_eq!(call.api_base, case.expected_base);
    assert_eq!(format!("{:?}", call.provider_config), case.config_name);
    assert_eq!(exposed(call.api_key.as_ref()), Some("sk-provider"));
    let openai_body = matches!(&call.body, ProviderRequestWire::OpenAi(_));
    assert_eq!(openai_body, case.provider == DecisionsProvider::Openai);
}

#[test]
fn prepare_call_success_systemone_fields_on_openai_provider() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder = CallBuilder::new(
        "openai/gpt-4o-mini",
        good_systemone_fields("Is the deploy green?"),
    );
    builder.api_key = Some(&key);
    let call = fixture.prepare(builder, None).unwrap();
    assert!(matches!(&call.request, RequestFormat::SystemOne(_)));
    let wire = openai_wire(&call.body);
    match &wire.input {
        InputWire::Text(text) => assert_eq!(text, "Is the deploy green?"),
        InputWire::Messages(_messages) => panic!("a SystemOne state becomes text input"),
    }
    assert_eq!(wire.questions.len(), 1);
}

#[test]
fn prepare_call_success_explicit_credentials_beat_globals() {
    let fixture = Fixture::with_environment(environment(&[("OPENAI_API_KEY", "sk-env")]));
    let key = secret("sk-explicit");
    let global = secret("sk-global");
    let mut builder =
        CallBuilder::new("openai/gpt-4o-mini", openai_predicate_fields("Is it true?"));
    builder.api_key = Some(&key);
    builder.api_base = Some("https://explicit.test");
    builder.globals = globals("https://globals.test", global);
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(call.api_base, "https://explicit.test");
    assert_eq!(exposed(call.api_key.as_ref()), Some("sk-explicit"));
}

#[test]
fn prepare_call_success_openai_globals_beat_environment() {
    let fixture = Fixture::with_environment(environment(&[
        ("OPENAI_API_KEY", "sk-env"),
        ("OPENAI_BASE_URL", "https://env.test"),
    ]));
    let global = secret("sk-global");
    let mut builder =
        CallBuilder::new("openai/gpt-4o-mini", openai_predicate_fields("Is it true?"));
    builder.globals = globals("https://globals.test", global);
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(call.api_base, "https://globals.test");
    assert_eq!(exposed(call.api_key.as_ref()), Some("sk-global"));
}

#[test]
fn prepare_call_success_openai_reads_environment() {
    let fixture = Fixture::with_environment(environment(&[
        ("OPENAI_API_KEY", "sk-env"),
        ("OPENAI_BASE_URL", "https://env.test"),
    ]));
    let builder = CallBuilder::new("openai/gpt-4o-mini", openai_predicate_fields("Is it true?"));
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(call.api_base, "https://env.test");
    assert_eq!(exposed(call.api_key.as_ref()), Some("sk-env"));
}

#[test]
fn prepare_call_success_cloudflare_account_id_builds_default_base() {
    let fixture = Fixture::with_environment(environment(&[
        ("CLOUDFLARE_ACCOUNT_ID", "acct-1"),
        ("CLOUDFLARE_API_KEY", "cf-key"),
    ]));
    let builder = CallBuilder::new("cloudflare/llama-3", good_systemone_fields("Is it green?"));
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(call.model, "@cf/cloudflare/llama-3");
    assert_eq!(
        call.api_base,
        "https://api.cloudflare.com/client/v4/accounts/acct-1/ai/run"
    );
    assert_eq!(exposed(call.api_key.as_ref()), Some("cf-key"));
    let url = call
        .provider_config
        .ops()
        .get_complete_url(&call.api_base, &call.model);
    assert_eq!(
        url,
        "https://api.cloudflare.com/client/v4/accounts/acct-1/ai/run/@cf/cloudflare/llama-3"
    );
}

#[test]
fn prepare_call_success_azure_reads_environment() {
    let fixture = Fixture::with_environment(environment(&[
        ("AZURE_AI_API_BASE", "https://env.services.ai.azure.com"),
        ("AZURE_AI_API_KEY", "azure-key"),
    ]));
    let builder = CallBuilder::new(
        "azure_ai/gpt-4o-mini",
        good_systemone_fields("Is it green?"),
    );
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(call.api_base, "https://env.services.ai.azure.com");
    assert_eq!(exposed(call.api_key.as_ref()), Some("azure-key"));
}

#[test]
fn prepare_call_success_hosted_vllm_needs_no_api_key() {
    let fixture = Fixture::default();
    let mut builder = CallBuilder::new(
        "hosted_vllm/my-model",
        good_systemone_fields("Is it green?"),
    );
    builder.api_base = Some("http://vllm.test:8000");
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(call.api_base, "http://vllm.test:8000");
    assert!(call.api_key.is_none());
}

#[test]
fn prepare_call_success_carries_headers_and_timeout() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut headers = Headers::new();
    headers.insert("X-Extra".to_owned(), "1".to_owned());
    let mut builder =
        CallBuilder::new("openai/gpt-4o-mini", openai_predicate_fields("Is it true?"));
    builder.api_key = Some(&key);
    builder.extra_headers = Some(&headers);
    builder.timeout = Some(Duration::from_secs(12));
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(call.headers.get("X-Extra").map(String::as_str), Some("1"));
    assert_eq!(call.timeout, Some(Duration::from_secs(12)));
}

#[test]
fn prepare_call_success_keeps_kwargs_as_litellm_params() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut kwargs = Extra::new();
    kwargs.insert(
        "litellm_call_id".to_owned(),
        Json::Text("call-9".to_owned()),
    );
    let mut builder = CallBuilder::new("typesafe/gpt-4o", good_systemone_fields("Is it green?"));
    builder.api_base = Some("https://typesafe.test");
    builder.api_key = Some(&key);
    builder.kwargs = kwargs;
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(
        call.litellm_params.get("litellm_call_id"),
        Some(&Json::Text("call-9".to_owned()))
    );
}

#[test]
fn prepare_call_success_updates_the_logging_state() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut kwargs = Extra::new();
    kwargs.insert(
        "litellm_call_id".to_owned(),
        Json::Text("call-7".to_owned()),
    );
    let mut builder =
        CallBuilder::new("openai/gpt-4o-mini", openai_predicate_fields("Is it true?"));
    builder.api_key = Some(&key);
    builder.kwargs = kwargs;
    let call = fixture.prepare(builder, Some(&fixture.logging)).unwrap();
    assert!(call.logging_obj.is_some());
    let state = fixture
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(state.model, "gpt-4o-mini");
    assert_eq!(state.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        state.litellm_params.get("api_base"),
        Some(&Json::Text(
            "https://api.openai.com/v1/decisions".to_owned()
        ))
    );
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&Json::Text("call-7".to_owned()))
    );
    assert!(state.optional_params.is_empty());
    assert_eq!(
        state.model_call_details.get("custom_llm_provider"),
        Some(&Json::Text("openai".to_owned()))
    );
}

#[test]
fn prepare_call_success_azure_uses_the_injected_route() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder = CallBuilder::new(
        "azure_ai/gpt-4o-mini",
        good_systemone_fields("Is it green?"),
    );
    builder.api_base = Some("https://res.services.ai.azure.com");
    builder.api_key = Some(&key);
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(
        call.provider_config
            .ops()
            .get_complete_url(&call.api_base, &call.model),
        "https://res.services.ai.azure.com/providers/microsoft/v1/systemone"
    );
    assert_eq!(exposed(call.api_key.as_ref()), Some("sk"));
}

#[test]
fn validation_requires_a_model_name() {
    let fixture = Fixture::default();
    let mut builder = CallBuilder::new("", good_systemone_fields("Is it green?"));
    builder.custom_llm_provider = Some("openai");
    let error = fixture.prepare(builder, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "A model name is required for the Decisions API"
    );
    let class = error.class();
    let ErrorClass::BadRequest(details) = &class else {
        panic!("expected a bad request, got {class:?}");
    };
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.model.as_deref(), Some(""));
}

#[test]
fn validation_unknown_provider_lists_supported_providers() {
    let fixture = Fixture::default();
    let mut builder = CallBuilder::new("claude-3-5-sonnet", good_systemone_fields("Is it green?"));
    builder.custom_llm_provider = Some("anthropic");
    let error = fixture.prepare(builder, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Unknown Decisions provider 'anthropic'. Supported providers: openai, openrouter, azure_ai, perplexity, typesafe, strands_decider, cloudflare, hosted_vllm"
    );
    assert!(matches!(error.class(), ErrorClass::BadRequest(_)));
}

#[test]
fn validation_unknown_provider_precedes_missing_model() {
    let fixture = Fixture::default();
    let mut builder = CallBuilder::new("", good_systemone_fields("Is it green?"));
    builder.custom_llm_provider = Some("anthropic");
    let error = fixture.prepare(builder, None).unwrap_err();
    assert!(matches!(&error, PrepareError::UnknownProvider { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Unknown Decisions provider 'anthropic'.")
    );
}

#[test]
fn validation_missing_provider_is_a_bad_request() {
    let fixture = Fixture::default();
    let builder = CallBuilder::new("gpt-4o-mini", good_systemone_fields("Is it green?"));
    let error = fixture.prepare(builder, None).unwrap_err();
    assert!(matches!(&error, PrepareError::Resolution { .. }));
    assert_eq!(
        error.to_string(),
        "LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model=gpt-4o-mini"
    );
    assert!(matches!(error.class(), ErrorClass::BadRequest(_)));
}

fn assert_invalid_request(fields: RequestFields, expected: &str) {
    let fixture = Fixture::default();
    let builder = CallBuilder::new("openai/gpt-4o-mini", fields);
    let error = fixture.prepare(builder, None).unwrap_err();
    assert!(matches!(&error, PrepareError::InvalidRequest { .. }));
    assert!(matches!(error.class(), ErrorClass::BadRequest(_)));
    assert!(error.to_string().starts_with("Invalid Decisions request: "));
    assert!(error.to_string().contains(expected));
}

#[rstest]
#[case::missing_state(RequestFields::SystemOne(SystemOneRequestFields { state: None, questions: Some(question_map(&[("first", noul_question())])) }), "DecisionsJson")]
#[case::missing_questions(RequestFields::SystemOne(SystemOneRequestFields { state: Some(Json::Text("state".to_owned())), questions: None }), "expected a map")]
#[case::empty_questions(RequestFields::SystemOne(SystemOneRequestFields { state: Some(Json::Text("state".to_owned())), questions: Some(IndexMap::new()) }), "expected 1 to 128 entries, found 0")]
fn validation_systemone_fields_are_required(#[case] fields: RequestFields, #[case] expected: &str) {
    assert_invalid_request(fields, expected);
}

#[test]
fn validation_systemone_question_names_are_required() {
    let fixture = Fixture::default();
    let builder = CallBuilder::new(
        "openai/gpt-4o-mini",
        RequestFields::SystemOne(SystemOneRequestFields {
            state: Some(Json::Text("state".to_owned())),
            questions: Some(question_map(&[("", noul_question())])),
        }),
    );
    let error = fixture.prepare(builder, None).unwrap_err();
    assert!(matches!(&error, PrepareError::InvalidRequest { .. }));
    assert!(matches!(error.class(), ErrorClass::BadRequest(_)));
    assert!(error.to_string().contains("expected 1 to"));
}

#[rstest]
#[case::missing_input(RequestFields::OpenAi(OpenAiDecisionRequestFields { input: None, questions: Some(vec![predicate_question()]) }), "DecisionInput")]
#[case::missing_questions(RequestFields::OpenAi(OpenAiDecisionRequestFields { input: Some(Json::Text("state".to_owned())), questions: None }), "expected a sequence")]
#[case::empty_questions(RequestFields::OpenAi(OpenAiDecisionRequestFields { input: Some(Json::Text("state".to_owned())), questions: Some(Vec::new()) }), "expected 1 to 128 entries, found 0")]
fn validation_openai_fields_are_required(#[case] fields: RequestFields, #[case] expected: &str) {
    assert_invalid_request(fields, expected);
}

#[rstest]
#[case::hosted_vllm(
    "hosted_vllm/my-model",
    "api_base is required for Decisions provider 'hosted_vllm'"
)]
#[case::cloudflare(
    "cloudflare/llama-3",
    "Missing CLOUDFLARE_ACCOUNT_ID - set CLOUDFLARE_ACCOUNT_ID or pass api_base explicitly"
)]
#[case::azure_ai(
    "azure_ai/gpt-4o-mini",
    "Missing AZURE_AI_API_BASE - set AZURE_AI_API_BASE or pass api_base, e.g. https://<resource>.services.ai.azure.com"
)]
fn validation_missing_api_base_uses_the_provider_message(
    #[case] model: &str,
    #[case] expected: &str,
) {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder = CallBuilder::new(model, good_systemone_fields("Is it green?"));
    builder.api_key = Some(&key);
    let error = fixture.prepare(builder, None).unwrap_err();
    assert!(matches!(&error, PrepareError::MissingApiBase { .. }));
    assert_eq!(error.to_string(), expected);
    let class = error.class();
    let ErrorClass::BadRequest(details) = &class else {
        panic!("expected a bad request, got {class:?}");
    };
    assert_eq!(details.model.as_deref(), Some(model));
}

#[test]
fn validation_missing_api_key_is_authentication() {
    let fixture = Fixture::default();
    let mut builder = CallBuilder::new("typesafe/gpt-4o", good_systemone_fields("Is it green?"));
    builder.api_base = Some("https://typesafe.test");
    let error = fixture.prepare(builder, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Missing API key for Decisions provider 'typesafe'"
    );
    let class = error.class();
    let ErrorClass::Authentication(details) = &class else {
        panic!("expected authentication, got {class:?}");
    };
    assert_eq!(details.status.0, 401);
    assert_eq!(details.provider.as_deref(), Some("typesafe"));
    assert_eq!(details.model.as_deref(), Some("typesafe/gpt-4o"));
}

#[test]
fn validation_empty_credentials_fall_through_to_the_environment() {
    let fixture = Fixture::with_environment(environment(&[("PERPLEXITY_API_KEY", "pplx-env")]));
    let empty_base = String::new();
    let empty_key = secret("");
    let mut builder = CallBuilder::new("perplexity/sonar", good_systemone_fields("Is it green?"));
    builder.api_base = Some(&empty_base);
    builder.api_key = Some(&empty_key);
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(call.api_base, "https://api.perplexity.ai");
    assert_eq!(exposed(call.api_key.as_ref()), Some("pplx-env"));
}

fn typesafe_selection(environment: &BTreeMap<String, String>) -> Selection<'_> {
    Selection {
        model: "typesafe/gpt-4o",
        environment,
        provider: DecisionsProvider::Typesafe,
        config: ProviderConfig::select(
            DecisionsProvider::Typesafe,
            LiteLlmGlobals::default(),
            environment,
            azure_ai(),
        ),
    }
}

#[test]
fn credentials_take_the_dynamic_values_before_the_explicit_ones() {
    let variables = environment(&[]);
    let selection = typesafe_selection(&variables);
    let dynamic_key = secret("sk-dynamic");
    let explicit_key = secret("sk-explicit");
    let resolved = credentials(
        &selection,
        &CredentialInput {
            api_base: Some("https://explicit.test"),
            api_key: Some(&explicit_key),
            dynamic_api_base: Some("https://dynamic.test"),
            dynamic_api_key: Some(&dynamic_key),
        },
    )
    .unwrap();
    assert_eq!(resolved.api_base, "https://dynamic.test");
    assert_eq!(exposed(resolved.api_key.as_ref()), Some("sk-dynamic"));
}

#[test]
fn credentials_empty_dynamic_values_fall_through_to_the_explicit_ones() {
    let variables = environment(&[]);
    let selection = typesafe_selection(&variables);
    let empty_base = String::new();
    let empty_key = secret("");
    let explicit_key = secret("sk-explicit");
    let resolved = credentials(
        &selection,
        &CredentialInput {
            api_base: Some("https://explicit.test"),
            api_key: Some(&explicit_key),
            dynamic_api_base: Some(&empty_base),
            dynamic_api_key: Some(&empty_key),
        },
    )
    .unwrap();
    assert_eq!(resolved.api_base, "https://explicit.test");
    assert_eq!(exposed(resolved.api_key.as_ref()), Some("sk-explicit"));
}

#[test]
fn provider_config_resolve_reads_the_provider_env_names() {
    let variables = environment(&[
        ("AZURE_AI_API_BASE", "https://env.services.ai.azure.com"),
        ("AZURE_AI_API_KEY", "azure-env-key"),
    ]);
    let config = ProviderConfig::select(
        DecisionsProvider::AzureAi,
        LiteLlmGlobals::default(),
        &variables,
        azure_ai(),
    );
    assert_eq!(
        config.resolve(Credential::Base, None, &variables),
        Some("https://env.services.ai.azure.com".to_owned())
    );
    assert_eq!(
        config.resolve(Credential::Key, None, &variables),
        Some("azure-env-key".to_owned())
    );
}

#[test]
fn errors_image_input_is_refused_by_a_systemone_provider() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder = CallBuilder::new("perplexity/sonar", openai_image_fields());
    builder.api_key = Some(&key);
    let error = fixture.prepare(builder, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Decisions provider 'perplexity' cannot serve this request: input_image content parts are not supported because System One providers accept text input only"
    );
    let cause = Error::source(&error).expect("the conversion error stays the source");
    assert!(cause.downcast_ref::<RequestConversionError>().is_some());
    let class = error.class();
    let ErrorClass::BadRequest(details) = &class else {
        panic!("expected a bad request, got {class:?}");
    };
    assert_eq!(details.provider.as_deref(), Some("perplexity"));
}

#[test]
fn errors_single_choice_is_refused_by_openai() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder = CallBuilder::new("openai/gpt-4o-mini", systemone_single_choice_fields());
    builder.api_key = Some(&key);
    let error = fixture.prepare(builder, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Decisions provider 'openai' cannot serve this request: OpenAI needs at least 2 choices or levels on every choice or score question"
    );
}

#[test]
fn errors_empty_question_name_has_no_systemone_key() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder = CallBuilder::new("perplexity/sonar", openai_empty_name_fields());
    builder.api_key = Some(&key);
    let error = fixture.prepare(builder, None).unwrap_err();
    let message = error.to_string();
    assert!(
        message.starts_with(
            "Decisions provider 'perplexity' cannot serve this request: expected 1 to "
        )
    );
    assert!(message.ends_with("entries, found 0"));
}

#[test]
fn safety_identifier_rejected_when_the_provider_does_not_support_it() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder = CallBuilder::new("typesafe/gpt-4o", openai_predicate_fields("Is it true?"));
    builder.api_base = Some("https://typesafe.test");
    builder.api_key = Some(&key);
    builder.safety_identifier = Some("user-1");
    let error = fixture.prepare(builder, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "typesafe does not support parameters: ['safety_identifier'], for model=typesafe/gpt-4o. To drop these, set `litellm.drop_params=True` or for proxy:\n\n`litellm_settings:\n drop_params: true`\n"
    );
    let class = error.class();
    let ErrorClass::UnsupportedParams(context) = &class else {
        panic!("expected unsupported params, got {class:?}");
    };
    assert_eq!(context.provider.as_deref(), Some("typesafe"));
    assert_eq!(context.model.as_deref(), Some("typesafe/gpt-4o"));
}

#[rstest]
#[case::global(true, None)]
#[case::call_bool(false, Some(Json::Bool(true)))]
#[case::call_text_true(false, Some(Json::Text("True".to_owned())))]
#[case::call_text_yes(false, Some(Json::Text("yes".to_owned())))]
#[case::call_number(false, Some(Json::Number(Number::from(1_u8))))]
fn safety_identifier_dropped_when_drop_params_is_on(
    #[case] global_drop_params: bool,
    #[case] drop_params: Option<Json>,
) {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder = CallBuilder::new("typesafe/gpt-4o", good_systemone_fields("Is it green?"));
    builder.api_base = Some("https://typesafe.test");
    builder.api_key = Some(&key);
    builder.safety_identifier = Some("user-1");
    builder.global_drop_params = global_drop_params;
    if let Some(value) = drop_params {
        builder.kwargs.insert("drop_params".to_owned(), value);
    }
    let call = fixture.prepare(builder, None).unwrap();
    assert!(call.ir_request.safety_identifier.is_none());
    assert_eq!(systemone_wire(&call.body).questions.len(), 1);
}

#[test]
fn safety_identifier_dropped_from_an_openai_request_body() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder = CallBuilder::new("typesafe/gpt-4o", openai_predicate_fields("Is it true?"));
    builder.api_base = Some("https://typesafe.test");
    builder.api_key = Some(&key);
    builder.safety_identifier = Some("user-1");
    builder.global_drop_params = true;
    let call = fixture.prepare(builder, None).unwrap();
    match &call.request {
        RequestFormat::OpenAi(body) => {
            assert_eq!(body.safety_identifier.as_deref(), Some("user-1"));
        }
        RequestFormat::SystemOne(_body) => panic!("expected an OpenAI body"),
    }
    assert!(call.ir_request.safety_identifier.is_none());
    assert_eq!(systemone_wire(&call.body).questions.len(), 1);
}

#[test]
fn safety_identifier_call_false_does_not_drop() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder = CallBuilder::new("typesafe/gpt-4o", good_systemone_fields("Is it green?"));
    builder.api_base = Some("https://typesafe.test");
    builder.api_key = Some(&key);
    builder.safety_identifier = Some("user-1");
    builder
        .kwargs
        .insert("drop_params".to_owned(), Json::Bool(false));
    let error = fixture.prepare(builder, None).unwrap_err();
    assert!(matches!(
        &error,
        PrepareError::UnsupportedSafetyIdentifier { .. }
    ));
}

#[test]
fn safety_identifier_global_drop_wins_over_call_false() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder = CallBuilder::new("typesafe/gpt-4o", good_systemone_fields("Is it green?"));
    builder.api_base = Some("https://typesafe.test");
    builder.api_key = Some(&key);
    builder.safety_identifier = Some("user-1");
    builder.global_drop_params = true;
    builder
        .kwargs
        .insert("drop_params".to_owned(), Json::Bool(false));
    let call = fixture.prepare(builder, None).unwrap();
    assert!(call.ir_request.safety_identifier.is_none());
}

#[test]
fn safety_identifier_kept_on_the_openai_provider() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder =
        CallBuilder::new("openai/gpt-4o-mini", openai_predicate_fields("Is it true?"));
    builder.api_key = Some(&key);
    builder.safety_identifier = Some("user-1");
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(call.ir_request.safety_identifier.as_deref(), Some("user-1"));
    assert_eq!(
        openai_wire(&call.body).safety_identifier.as_deref(),
        Some("user-1")
    );
}

#[test]
fn safety_identifier_reaches_systemone_ir_on_a_provider_that_takes_it() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder = CallBuilder::new(
        "openai/gpt-4o-mini",
        good_systemone_fields("Is the deploy green?"),
    );
    builder.api_key = Some(&key);
    builder.safety_identifier = Some("user-1");
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(call.ir_request.safety_identifier.as_deref(), Some("user-1"));
    assert_eq!(
        openai_wire(&call.body).safety_identifier.as_deref(),
        Some("user-1")
    );
}

#[rstest]
#[case::input("input", "decisions")]
#[case::state("state", "systemone")]
fn reject_format_rejects_the_other_formats_field(
    #[case] field: &str,
    #[case] other_function: &str,
) {
    let mut kwargs = Extra::new();
    kwargs.insert(field.to_owned(), Json::Null);
    let error = reject_other_format("gpt-4o-mini", &kwargs, field, other_function).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Requests with '{field}' go through litellm.{other_function}")
    );
    let class = error.class();
    let ErrorClass::BadRequest(details) = &class else {
        panic!("expected a bad request, got {class:?}");
    };
    assert_eq!(details.provider.as_deref(), Some(""));
    assert_eq!(details.model.as_deref(), Some("gpt-4o-mini"));
}

#[test]
fn reject_format_accepts_a_request_without_the_field() {
    reject_other_format("gpt-4o-mini", &Extra::new(), "input", "decisions").unwrap();
}

#[test]
fn reject_format_rejects_a_null_field() {
    let mut kwargs = Extra::new();
    kwargs.insert("input".to_owned(), Json::Null);
    let error = reject_other_format("gpt-4o-mini", &kwargs, "input", "decisions").unwrap_err();
    assert!(matches!(&error, PrepareError::OtherFormat { .. }));
}

#[test]
fn request_format_validates_a_systemone_body() {
    let fields = good_systemone_fields("Is it green?");
    match fields.validate(None).unwrap() {
        RequestFormat::SystemOne(body) => {
            assert_eq!(body.state, DecisionsJson::Text("Is it green?".to_owned()));
            assert_eq!(body.questions.as_ref().len(), 1);
        }
        RequestFormat::OpenAi(_body) => panic!("expected a SystemOne body"),
    }
}

fn assert_rejected_fields(fields: RequestFields, expected: &str) {
    let error = fields.validate(None).unwrap_err();
    assert!(
        error.to_string().contains(expected),
        "expected {expected} in {error}"
    );
}

#[rstest]
#[case::missing_state(RequestFields::SystemOne(SystemOneRequestFields { state: None, questions: Some(question_map(&[("first", noul_question())])) }), "DecisionsJson")]
#[case::missing_questions(RequestFields::SystemOne(SystemOneRequestFields { state: Some(Json::Text("state".to_owned())), questions: None }), "expected a map")]
#[case::empty_questions(RequestFields::SystemOne(SystemOneRequestFields { state: Some(Json::Text("state".to_owned())), questions: Some(IndexMap::new()) }), "expected 1 to 128 entries, found 0")]
fn request_format_systemone_rejects_an_incomplete_body(
    #[case] fields: RequestFields,
    #[case] expected: &str,
) {
    assert_rejected_fields(fields, expected);
}

#[test]
fn request_format_systemone_rejects_a_blank_question_name() {
    let fields = RequestFields::SystemOne(SystemOneRequestFields {
        state: Some(Json::Text("state".to_owned())),
        questions: Some(question_map(&[("", noul_question())])),
    });
    let error = fields.validate(None).unwrap_err();
    assert!(error.to_string().contains("expected 1 to"));
}

#[test]
fn request_format_validates_an_openai_body() {
    let fields = openai_predicate_fields("Is it true?");
    match fields.validate(Some("user-1")).unwrap() {
        RequestFormat::OpenAi(body) => {
            assert_eq!(body.safety_identifier.as_deref(), Some("user-1"));
            assert_eq!(body.questions.as_ref().len(), 1);
        }
        RequestFormat::SystemOne(_body) => panic!("expected an OpenAI body"),
    }
}

#[rstest]
#[case::missing_input(RequestFields::OpenAi(OpenAiDecisionRequestFields { input: None, questions: Some(vec![predicate_question()]) }), "DecisionInput")]
#[case::missing_questions(RequestFields::OpenAi(OpenAiDecisionRequestFields { input: Some(Json::Text("state".to_owned())), questions: None }), "expected a sequence")]
#[case::empty_questions(RequestFields::OpenAi(OpenAiDecisionRequestFields { input: Some(Json::Text("state".to_owned())), questions: Some(Vec::new()) }), "expected 1 to 128 entries, found 0")]
fn request_format_openai_rejects_an_incomplete_body(
    #[case] fields: RequestFields,
    #[case] expected: &str,
) {
    assert_rejected_fields(fields, expected);
}

#[test]
fn request_format_openai_rejects_an_unknown_message_field() {
    let fields = RequestFields::OpenAi(OpenAiDecisionRequestFields {
        input: Some(json(
            r#"[{"role":"user","type":"message","content":"hi","unexpected":1}]"#,
        )),
        questions: Some(vec![predicate_question()]),
    });
    let error = fields.validate(None).unwrap_err();
    assert!(error.to_string().contains("DecisionInput"));
}

#[test]
fn request_format_openai_rejects_a_single_choice() {
    let fields = RequestFields::OpenAi(OpenAiDecisionRequestFields {
        input: Some(Json::Text("state".to_owned())),
        questions: Some(vec![json(
            r#"{"type":"choice","name":"pick","instructions":"Which?","choices":[{"value":"a"}]}"#,
        )]),
    });
    let error = fields.validate(None).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("expected 2 to 255 entries, found 1")
    );
}

#[test]
fn fields_carry_every_decisions_call_value() {
    let fixture = Fixture::default();
    let key = secret("sk-carry");
    let mut headers = Headers::new();
    headers.insert("X-Extra".to_owned(), "1".to_owned());
    let mut kwargs = Extra::new();
    kwargs.insert(
        "litellm_call_id".to_owned(),
        Json::Text("call-5".to_owned()),
    );
    let mut builder =
        CallBuilder::new("openai/gpt-4o-mini", openai_predicate_fields("Is it true?"));
    builder.api_key = Some(&key);
    builder.api_base = Some("https://openai.test");
    builder.extra_headers = Some(&headers);
    builder.timeout = Some(Duration::from_secs(5));
    builder.safety_identifier = Some("user-1");
    builder.kwargs = kwargs;
    let call = fixture.prepare(builder, Some(&fixture.logging)).unwrap();
    assert_eq!(call.model, "gpt-4o-mini");
    assert_eq!(call.requested_model, "openai/gpt-4o-mini");
    assert_eq!(call.custom_llm_provider, "openai");
    assert!(matches!(&call.request, RequestFormat::OpenAi(_)));
    assert_eq!(call.ir_request.safety_identifier.as_deref(), Some("user-1"));
    assert_eq!(openai_wire(&call.body).model, "gpt-4o-mini");
    assert_eq!(call.api_base, "https://openai.test");
    assert_eq!(exposed(call.api_key.as_ref()), Some("sk-carry"));
    assert!(call.logging_obj.is_some());
    assert_eq!(call.headers.get("X-Extra").map(String::as_str), Some("1"));
    assert_eq!(call.timeout, Some(Duration::from_secs(5)));
    assert_eq!(
        call.litellm_params.get("litellm_call_id"),
        Some(&Json::Text("call-5".to_owned()))
    );
    assert_eq!(format!("{:?}", call.provider_config), "Openai");
}

#[test]
fn fields_default_to_empty_headers_and_no_logging() {
    let fixture = Fixture::default();
    let mut builder = CallBuilder::new(
        "hosted_vllm/my-model",
        good_systemone_fields("Is it green?"),
    );
    builder.api_base = Some("http://vllm.test:8000");
    let call = fixture.prepare(builder, None).unwrap();
    assert!(call.headers.is_empty());
    assert!(call.logging_obj.is_none());
    assert!(call.timeout.is_none());
    assert!(call.api_key.is_none());
}

#[test]
fn fields_provider_config_answers_its_own_questions() {
    let fixture = Fixture::default();
    let key = secret("sk");
    let mut builder =
        CallBuilder::new("openai/gpt-4o-mini", openai_predicate_fields("Is it true?"));
    builder.api_key = Some(&key);
    let call = fixture.prepare(builder, None).unwrap();
    assert_eq!(
        call.provider_config.ops().canonical_model("gpt-4o-mini"),
        "gpt-4o-mini"
    );
    assert_eq!(
        call.provider_config
            .ops()
            .get_complete_url("https://api.openai.com", "gpt-4o-mini"),
        "https://api.openai.com/v1/decisions"
    );
    assert!(call.provider_config.ops().api_key_required());
    assert!(call.provider_config.ops().supports_safety_identifier());
    assert_eq!(
        call.provider_config
            .ops()
            .missing_api_base_message("openai"),
        "api_base is required for Decisions provider 'openai'"
    );
    let mut hosted_builder = CallBuilder::new(
        "hosted_vllm/my-model",
        good_systemone_fields("Is it green?"),
    );
    hosted_builder.api_base = Some("http://vllm.test:8000");
    let hosted = fixture.prepare(hosted_builder, None).unwrap();
    assert!(!hosted.provider_config.ops().api_key_required());
    assert!(!hosted.provider_config.ops().supports_safety_identifier());
}

#[test]
fn fields_debug_never_prints_the_api_key() {
    let fixture = Fixture::default();
    let key = secret("sk-debug");
    let mut builder =
        CallBuilder::new("openai/gpt-4o-mini", openai_predicate_fields("Is it true?"));
    builder.api_key = Some(&key);
    builder.kwargs.insert(
        "aws_secret_access_key".to_owned(),
        Json::Text("aws-secret-key".to_owned()),
    );
    let prepare_debug = fixture.debug_prepare_call(builder);
    assert!(prepare_debug.contains("PrepareCall"));
    assert!(!prepare_debug.contains("sk-debug"));
    assert!(!prepare_debug.contains("aws-secret-key"));
    let mut call_builder =
        CallBuilder::new("openai/gpt-4o-mini", openai_predicate_fields("Is it true?"));
    call_builder.api_key = Some(&key);
    call_builder.kwargs.insert(
        "aws_secret_access_key".to_owned(),
        Json::Text("aws-secret-key".to_owned()),
    );
    let call = fixture.prepare(call_builder, None).unwrap();
    assert_eq!(
        call.litellm_params.get("aws_secret_access_key"),
        Some(&Json::Text("aws-secret-key".to_owned()))
    );
    let call_debug = format!("{call:?}");
    assert!(call_debug.contains("DecisionsCall"));
    assert!(call_debug.contains("<redacted>"));
    assert!(!call_debug.contains("sk-debug"));
    assert!(!call_debug.contains("aws-secret-key"));
    assert!(format!("{:?}", azure_ai()).contains("AzureAiConfig"));
}
