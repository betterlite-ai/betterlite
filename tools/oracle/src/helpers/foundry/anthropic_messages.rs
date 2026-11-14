use std::collections::BTreeMap;

use betterlite_config::sections::Section;
use betterlite_config::sections::azure_credentials::{DeploymentSettings, Raw as TokenRaw};
use betterlite_config::sections::azure_url_policy::Raw as KeyRaw;
use betterlite_core::api::anthropic_beta_filter::error::BetaFilterError;
use betterlite_core::api::anthropic_beta_filter::native_injection::{
    NativeRequest, inject_native_betas,
};
use betterlite_foundry::anthropic_messages::environment::{
    EnvironmentRequest, ParamsKey, TokenSource, validate_anthropic_messages_environment,
};
use betterlite_foundry::anthropic_messages::environment_error::EnvironmentError;
use betterlite_foundry::anthropic_messages::transform::transform_anthropic_messages_request;
use betterlite_foundry::anthropic_messages::url::get_complete_url;
use betterlite_foundry::anthropic_messages::{CUSTOM_LLM_PROVIDER, SHOULD_STRIP_BILLING_METADATA};
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod input;
mod sources;
mod tests;

use input::{EnvironmentStep, Globals, Input};
use sources::Offline;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Url {
                api_bases,
                environment,
            } => url(api_bases, environment),
            Input::Environment { calls } => environment(calls),
            Input::Transform { request } => Ok(json!({ "request": transform(request) })),
            Input::Projection { request } => Ok(json!({
                "custom_llm_provider": CUSTOM_LLM_PROVIDER,
                "should_strip_billing_metadata": SHOULD_STRIP_BILLING_METADATA,
                "request": transform(request),
            })),
        }
    }
}

fn url(api_bases: &[Option<String>], environment: &BTreeMap<String, String>) -> HelperResult {
    let from_environment = environment.get("AZURE_API_BASE").map(String::as_str);
    let mut urls = Vec::with_capacity(api_bases.len());
    for api_base in api_bases {
        urls.push(get_complete_url(api_base.as_deref(), from_environment).or_value_error()?);
    }
    Ok(json!({ "urls": urls }))
}

fn transform(request: &Object) -> Object {
    let mut body = request.clone();
    transform_anthropic_messages_request(&mut body);
    body
}

// litellm.api_key, litellm.azure_key and the Azure key variables, through the production config section
fn configured_key(
    globals: &Globals,
    environment: &BTreeMap<String, String>,
) -> Result<Option<SecretString>, Box<HelperError>> {
    let source = json!({
        "litellm_settings": {"api_key": globals.api_key, "azure_key": globals.azure_key},
        "model_list": [{"litellm_params": {}}],
    });
    let raw: KeyRaw = serde_json::from_value(source).or_config_error()?;
    let resolved = Section::convert(raw, environment).or_config_error()?;
    Ok(resolved
        .deployments
        .into_iter()
        .next()
        .and_then(|deployment| deployment.api_key))
}

fn token_settings(
    litellm_params: &BTreeMap<String, Option<String>>,
    environment: &BTreeMap<String, String>,
) -> Result<DeploymentSettings, Box<HelperError>> {
    let source = json!({
        "litellm_settings": {"enable_azure_ad_token_refresh": false},
        "model_list": [{"litellm_params": litellm_params}],
    });
    let raw: TokenRaw = serde_json::from_value(source).or_config_error()?;
    let resolved = Section::convert(raw, environment).or_config_error()?;
    resolved.deployments.into_iter().next().ok_or_else(|| {
        Box::new(HelperError {
            class: "ConfigError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "no deployment resolved".to_owned(),
        })
    })
}

fn environment(calls: &[EnvironmentStep]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for step in calls {
        answers.push(environment_answer(step)?);
    }
    Ok(json!({ "calls": answers }))
}

fn key_shape<'data>(
    litellm_params: &'data BTreeMap<String, Option<String>>,
    secret: &'data Option<SecretString>,
) -> ParamsKey<'data> {
    match (litellm_params.get("api_key"), secret.as_ref()) {
        (None, _) => ParamsKey::DictAbsent,
        (Some(_), None) => ParamsKey::DictNull,
        (Some(_), Some(value)) => ParamsKey::Dict(value),
    }
}

fn environment_answer(step: &EnvironmentStep) -> Result<Value, Box<HelperError>> {
    let params_secret = step
        .litellm_params
        .get("api_key")
        .and_then(|entry| entry.as_deref())
        .map(SecretString::from);
    let params_key = key_shape(&step.litellm_params, &params_secret);
    let configured = configured_key(&step.globals, &step.environment)?;
    let credentials = token_settings(&step.litellm_params, &step.environment)?;
    let argument = step.api_key.as_deref().map(SecretString::from);
    let request = EnvironmentRequest {
        headers: step.headers.clone(),
        params_key,
        argument_key: argument.as_ref(),
        configured_key: configured.as_ref(),
        token: TokenSource {
            credentials: &credentials,
            refresh_enabled: false,
            provider: None,
            sources: &Offline,
        },
        api_base: step.api_base.clone(),
    };
    // LiteLLM calls the beta updater without a provider, so it runs under its default one
    let native = NativeRequest {
        provider: "anthropic",
        optional_params: &step.optional_params,
        messages: &step.messages,
    };
    let (headers, api_base) = validate_anthropic_messages_environment(request, |updated| {
        inject_native_betas(updated.into_iter().collect(), &native)
            .map(|merged| merged.into_iter().collect())
    })
    .map_err(helper_environment_error)?;
    Ok(json!({ "headers": headers, "api_base": api_base }))
}

fn helper_environment_error(error: EnvironmentError<BetaFilterError>) -> Box<HelperError> {
    match error {
        EnvironmentError::Token(token) => Box::new(HelperError {
            class: token.class().to_owned(),
            status: token.status(),
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: token.to_string(),
        }),
        EnvironmentError::Betas(betas) => Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: betas.to_string(),
        }),
    }
}
