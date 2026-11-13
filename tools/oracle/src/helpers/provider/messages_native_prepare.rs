use std::collections::BTreeMap;

use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_config::sections::anthropic_credentials::Settings as ConfigSettings;
use betterlite_core::api::anthropic_beta_filter::native_injection::NativeRequest;
use betterlite_core::api::messages_native_prepare::billing::remove_cache_scope;
use betterlite_core::api::messages_native_prepare::environment::validate_environment;
use betterlite_core::api::messages_native_prepare::fields::native_filters_betas;
use betterlite_core::api::messages_native_prepare::system_turns::normalize_system_turns;
use betterlite_core::api::messages_native_prepare::url::messages_url;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::api::messages_native_prepare::error::MessagesContractError;
use betterlite_provider::api::messages_native_prepare::{AnthropicMessagesConfig, Headers};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    SupportedFields {
        providers: Vec<Option<ProviderId>>,
    },
    Url {
        api_bases: Vec<Option<String>>,
        environment: BTreeMap<String, String>,
    },
    Environment {
        calls: Vec<EnvironmentCall>,
    },
    Transform {
        calls: Vec<TransformCall>,
    },
    BaseDefaults(BaseDefaultsCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentCall {
    headers: IndexMap<String, String>,
    optional_params: Object,
    messages: Vec<Json>,
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformCall {
    request: Object,
    supports_mid_system: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BaseDefaultsCall {
    headers: IndexMap<String, String>,
    status: u16,
    text: String,
    request: Object,
    error_message: String,
    error_status: u16,
}

// A subclass that implements nothing: every default comes from the base contract
struct Base;

impl AnthropicMessagesConfig for Base {
    type Error = MessagesContractError;
}

// A subclass that only names its provider, so the contract defaults answer for it
struct Contract {
    provider: Option<ProviderId>,
}

impl AnthropicMessagesConfig for Contract {
    type Error = MessagesContractError;

    fn custom_llm_provider(&self) -> Option<ProviderId> {
        self.provider
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::SupportedFields { providers } => Ok(supported_fields(providers)),
            Input::Url {
                api_bases,
                environment,
            } => url(api_bases, environment),
            Input::Environment { calls } => environment(calls),
            Input::Transform { calls } => Ok(transform(calls)),
            Input::BaseDefaults(case) => Ok(base_defaults(case)),
        }
    }
}

fn supported_fields(providers: &[Option<ProviderId>]) -> Value {
    let answers: Vec<Value> = providers
        .iter()
        .map(|provider| {
            let contract = Contract {
                provider: *provider,
            };
            json!({
                "fields": contract.supported_params("claude-sonnet-4-5"),
                "filters_betas": native_filters_betas(*provider),
                "strips_billing": contract.strips_billing_metadata(),
            })
        })
        .collect();
    json!({ "providers": answers })
}

fn settings(environment: &BTreeMap<String, String>) -> Result<AnthropicSettings, Box<HelperError>> {
    let resolved = ConfigSettings::resolve_environment(environment).map_err(failure)?;
    Ok(AnthropicSettings {
        api_key: resolved.api_key,
        auth_token: resolved.auth_token,
        api_base: resolved.api_base,
        use_bearer_for_custom_base: resolved.use_bearer_for_custom_base,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    })
}

fn url(api_bases: &[Option<String>], environment: &BTreeMap<String, String>) -> HelperResult {
    let resolved = settings(environment)?;
    let urls: Vec<String> = api_bases
        .iter()
        .map(|api_base| messages_url(api_base.as_deref(), &resolved))
        .collect();
    Ok(json!({ "urls": urls }))
}

fn environment(calls: &[EnvironmentCall]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let settings = settings(&call.environment)?;
        answers.push(environment_answer(call, &settings)?);
    }
    Ok(json!({ "calls": answers }))
}

fn environment_answer(
    call: &EnvironmentCall,
    settings: &AnthropicSettings,
) -> Result<Value, Box<HelperError>> {
    let mut headers: Headers = call.headers.clone();
    let request = NativeRequest {
        provider: "anthropic",
        optional_params: &call.optional_params,
        messages: &call.messages,
    };
    let api_base =
        validate_environment(&mut headers, None, None, &request, settings).map_err(|error| {
            Box::new(HelperError {
                class: error.class_name().to_owned(),
                status: error.status_code(),
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    Ok(json!({ "headers": headers, "api_base": api_base }))
}

fn transform(calls: &[TransformCall]) -> Value {
    let answers: Vec<Value> = calls
        .iter()
        .map(|call| {
            let mut body = call.request.clone();
            normalize_system_turns(&mut body, call.supports_mid_system);
            remove_cache_scope(&mut body);
            json!({
                "system": body.get("system"),
                "messages": body.get("messages"),
                "tools": body.get("tools"),
            })
        })
        .collect();
    json!({ "calls": answers })
}

fn base_defaults(case: &BaseDefaultsCall) -> Value {
    let signed = Base.sign_request(case.headers.clone());
    let mut updated = case.request.clone();
    Base.request_after_http_error(case.status, &case.text, &mut updated);
    let error = Base.error_class(&case.error_message, case.error_status, Headers::new());
    json!({
        "signed": {
            "headers": signed.headers,
            "has_signed_body": signed.signed_body.is_some(),
        },
        "web_search": Base.handles_web_search_natively(),
        "uses_provider_base": Base.uses_provider_api_base(),
        "filters_betas": Base.filters_beta_headers(),
        "error": {"status": error.status_code, "message": error.message},
        "max_retries": Base.max_http_retries(),
        "retried": Base.retries_http_error(case.status, &case.text),
        "request": updated,
    })
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: std::fmt::Display,
{
    Box::new(HelperError {
        class: "Error".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
