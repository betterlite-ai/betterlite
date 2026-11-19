use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::json_messages_policy::{
    JsonMessagesPolicy, MessagesCredentials, generic_complete_url, generic_transform_request,
    generic_validate_environment,
};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{JSONProviderRegistry, ProviderDescriptor};
use betterlite_openai::json_provider_policy::{
    OpenAiLikeResponsesPolicy, ResponsesPolicy, ResponsesPolicyCache,
};
use betterlite_openai::openai_base_config::Headers as OpenAiHeaders;
use betterlite_openai::response_input::ResponsesInput;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod capabilities;
mod cases;
mod tests;

use self::capabilities::NoCapabilities;
use self::cases::{Call, CaseInput, Kind};

pub(crate) struct Op;

impl Helper for Op {
    type Input = CaseInput;

    fn run(input: &Self::Input) -> HelperResult {
        let registry = JSONProviderRegistry::load();
        let mut results: Vec<Value> = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(run_call(call, &registry)?);
        }
        Ok(Value::Array(results))
    }
}

fn run_call(call: &Call, registry: &JSONProviderRegistry) -> Result<Value, Box<HelperError>> {
    let (slug, model) = split_model(&call.model)?;
    let provider = provider_id(&slug)?;
    match call.kind {
        Kind::GenericMessagesUrl => {
            let url = generic_complete_url(call.api_base.as_deref())
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            Ok(Value::String(url))
        }
        Kind::JsonMessagesUrl => {
            let policy = descriptor_for(call, registry, provider)
                .map(|descriptor| JsonMessagesPolicy { descriptor })?;
            let (explicit, global) = keys(call);
            let credentials = MessagesCredentials {
                api_base: call.api_base.as_deref(),
                api_key: explicit.as_ref(),
                global_api_key: global.as_ref(),
                lookup: &call.environment,
            };
            let url = policy
                .complete_url(&credentials)
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            Ok(Value::String(url))
        }
        Kind::GenericMessagesHeaders => {
            let (explicit, _) = keys(call);
            let headers = generic_validate_environment(
                call.headers.clone(),
                explicit.as_ref(),
                &call.optional_params,
                &call.messages,
            )
            .map_err(|error| helper_error("ValueError", error.to_string()))?;
            serde_json::to_value(headers)
        }
        Kind::JsonMessagesHeaders => {
            let policy = descriptor_for(call, registry, provider)
                .map(|descriptor| JsonMessagesPolicy { descriptor })?;
            let (explicit, global) = keys(call);
            let credentials = MessagesCredentials {
                api_base: call.api_base.as_deref(),
                api_key: explicit.as_ref(),
                global_api_key: global.as_ref(),
                lookup: &call.environment,
            };
            let headers = policy
                .validate_environment(
                    call.headers.clone(),
                    &credentials,
                    &call.optional_params,
                    &call.messages,
                )
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            serde_json::to_value(headers)
        }
        Kind::GenericMessagesTransform => {
            let runtime = runtime()?;
            let body = runtime
                .block_on(generic_transform_request(
                    &mut NoCapabilities,
                    &model,
                    call.messages.clone(),
                    call.optional_params.clone(),
                ))
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            serde_json::to_value(body)
        }
        Kind::JsonMessagesTransform => {
            let policy = descriptor_for(call, registry, provider)
                .map(|descriptor| JsonMessagesPolicy { descriptor })?;
            let runtime = runtime()?;
            let body = runtime
                .block_on(policy.transform_request(
                    &mut NoCapabilities,
                    &model,
                    call.messages.clone(),
                    call.optional_params.clone(),
                ))
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            serde_json::to_value(body)
        }
        Kind::ResponsesUrl => {
            let policy = descriptor_for(call, registry, provider)
                .map(|descriptor| ResponsesPolicy { descriptor })?;
            Ok(Value::String(
                policy.complete_url(call.api_base.as_deref(), &call.environment),
            ))
        }
        Kind::ResponsesHeaders => {
            let policy = descriptor_for(call, registry, provider)
                .map(|descriptor| ResponsesPolicy { descriptor })?;
            let (explicit, _) = keys(call);
            let headers = policy.validate_environment(
                responses_headers(call),
                explicit.as_ref(),
                &call.environment,
            );
            serde_json::to_value(headers)
        }
        Kind::ResponsesTransform => {
            let policy = descriptor_for(call, registry, provider)
                .map(|descriptor| ResponsesPolicy { descriptor })?;
            let body = policy
                .transform_request(
                    &model,
                    &ResponsesInput::Text(call.input.clone()),
                    &call.params,
                    None,
                )
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            serde_json::to_value(body)
        }
        Kind::ResponsesCache => cache_value(call, &slug, provider),
        Kind::GenericResponsesUrl => {
            let settings = openai_like_settings(call)?;
            let url = OpenAiLikeResponsesPolicy { settings }
                .complete_url(call.api_base.as_deref())
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            Ok(Value::String(url))
        }
        Kind::GenericResponsesHeaders => {
            let settings = openai_like_settings(call)?;
            let (explicit, _) = keys(call);
            let headers = OpenAiLikeResponsesPolicy { settings }
                .validate_environment(responses_headers(call), explicit.as_ref());
            serde_json::to_value(headers)
        }
    }
}

fn split_model(model: &str) -> Result<(String, String), Box<HelperError>> {
    let Some((slug, rest)) = model.split_once('/') else {
        return Err(helper_error(
            "ValueError",
            format!("the case model {model} carries no provider prefix"),
        ));
    };
    Ok((slug.to_owned(), rest.to_owned()))
}

fn provider_id(slug: &str) -> Result<ProviderId, Box<HelperError>> {
    slug.parse()
        .map_err(|error| helper_error("ValueError", format!("{slug}: {error}")))
}

fn keys(call: &Call) -> (Option<SecretString>, Option<SecretString>) {
    (
        call.api_key.clone().map(SecretString::from),
        call.global_api_key.clone().map(SecretString::from),
    )
}

// the Responses policies carry the OpenAI crate's sorted header map
fn responses_headers(call: &Call) -> OpenAiHeaders {
    call.headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn descriptor_for(
    call: &Call,
    registry: &JSONProviderRegistry,
    provider: ProviderId,
) -> Result<ProviderDescriptor, Box<HelperError>> {
    match &call.descriptor {
        Some(config) => Ok(ProviderDescriptor {
            provider,
            config: config.clone(),
        }),
        None => registry.providers.get(&provider).cloned().ok_or_else(|| {
            helper_error("ValueError", format!("{provider} is not a JSON provider"))
        }),
    }
}

// create_responses_config_class caches by slug for the process, so the first descriptor's class answers both
fn cache_value(call: &Call, slug: &str, provider: ProviderId) -> Result<Value, Box<HelperError>> {
    let first = call.first.clone().ok_or_else(|| {
        helper_error(
            "ValueError",
            format!("the {slug} cache call needs two descriptors"),
        )
    })?;
    let second = call.second.clone().ok_or_else(|| {
        helper_error(
            "ValueError",
            format!("the {slug} cache call needs two descriptors"),
        )
    })?;
    let lookup = BTreeMap::<String, String>::new();
    let mut cache = ResponsesPolicyCache::default();
    let first_url = cache
        .policy_for_descriptor(ProviderDescriptor {
            provider,
            config: first,
        })
        .complete_url(None, &lookup);
    let second_url = cache
        .policy_for_descriptor(ProviderDescriptor {
            provider,
            config: second,
        })
        .complete_url(None, &lookup);
    Ok(json!({
        "first": first_url,
        "second": second_url,
        "cached": cache.policies.len() == 1,
    }))
}

fn openai_like_settings(
    call: &Call,
) -> Result<betterlite_config::sections::openai_like_settings::Settings, Box<HelperError>> {
    ConfigLoader {
        lookup: call.environment.clone(),
    }
    .load("")
    .map(|config| config.sections.openai_like_settings)
    .map_err(|error| helper_error("ValueError", error.to_string()))
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("RuntimeError", error.to_string()))
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
