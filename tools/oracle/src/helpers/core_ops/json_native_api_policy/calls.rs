use std::collections::BTreeMap;

use betterlite_core::json_messages_policy::{
    JsonMessagesPolicy, MessagesCredentials, generic_complete_url, generic_transform_request,
    generic_validate_environment,
};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{JSONProviderRegistry, ProviderDescriptor};
use betterlite_openai::json_provider_policy::responses::{
    OpenAiLikeResponsesPolicy, ResponsesPolicy, ResponsesPolicyCache,
};
use betterlite_openai::response_input::ResponsesInput;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;

use super::capabilities::NoCapabilities;
use super::cases::{Call, Kind};
use super::{
    descriptor_for, helper_error, keys, openai_like_settings, provider_id, runtime, sorted_headers,
    split_model,
};

pub(super) fn run_call(
    call: &Call,
    registry: &JSONProviderRegistry,
) -> Result<Value, Box<HelperError>> {
    let (slug, model) = split_model(&call.model)?;
    let provider = provider_id(&slug)?;
    match call.kind {
        Kind::GenericMessagesUrl => generic_messages_url(call),
        Kind::JsonMessagesUrl => json_messages_url(call, registry, provider),
        Kind::GenericMessagesHeaders => generic_messages_headers(call),
        Kind::JsonMessagesHeaders => json_messages_headers(call, registry, provider),
        Kind::GenericMessagesTransform => generic_messages_transform(call, &model),
        Kind::JsonMessagesTransform => json_messages_transform(call, registry, provider, &model),
        Kind::ResponsesUrl => responses_url(call, registry, provider),
        Kind::ResponsesHeaders => responses_headers(call, registry, provider),
        Kind::ResponsesTransform => responses_transform(call, registry, provider, &model),
        Kind::ResponsesCache => responses_cache(call, &slug, provider),
        Kind::GenericResponsesUrl => generic_responses_url(call),
        Kind::GenericResponsesHeaders => generic_responses_headers(call),
    }
}

fn generic_messages_url(call: &Call) -> Result<Value, Box<HelperError>> {
    let url = generic_complete_url(call.api_base.as_deref())
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    Ok(Value::String(url))
}

fn json_messages_url(
    call: &Call,
    registry: &JSONProviderRegistry,
    provider: ProviderId,
) -> Result<Value, Box<HelperError>> {
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

fn generic_messages_headers(call: &Call) -> Result<Value, Box<HelperError>> {
    let (explicit, _) = keys(call);
    let headers = generic_validate_environment(
        call.headers.clone(),
        explicit.as_ref(),
        &call.optional_params,
        &call.messages,
    )
    .map_err(|error| helper_error("ValueError", error.to_string()))?;
    serde_json::to_value(headers).map_err(|error| helper_error("ValueError", error.to_string()))
}

fn json_messages_headers(
    call: &Call,
    registry: &JSONProviderRegistry,
    provider: ProviderId,
) -> Result<Value, Box<HelperError>> {
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
    serde_json::to_value(headers).map_err(|error| helper_error("ValueError", error.to_string()))
}

fn generic_messages_transform(call: &Call, model: &str) -> Result<Value, Box<HelperError>> {
    let runtime = runtime()?;
    let body = runtime
        .block_on(generic_transform_request(
            &mut NoCapabilities,
            model,
            call.messages.clone(),
            call.optional_params.clone(),
        ))
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    serde_json::to_value(body).map_err(|error| helper_error("ValueError", error.to_string()))
}

fn json_messages_transform(
    call: &Call,
    registry: &JSONProviderRegistry,
    provider: ProviderId,
    model: &str,
) -> Result<Value, Box<HelperError>> {
    let policy = descriptor_for(call, registry, provider)
        .map(|descriptor| JsonMessagesPolicy { descriptor })?;
    let runtime = runtime()?;
    let body = runtime
        .block_on(policy.transform_request(
            &mut NoCapabilities,
            model,
            call.messages.clone(),
            call.optional_params.clone(),
        ))
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    serde_json::to_value(body).map_err(|error| helper_error("ValueError", error.to_string()))
}

fn responses_url(
    call: &Call,
    registry: &JSONProviderRegistry,
    provider: ProviderId,
) -> Result<Value, Box<HelperError>> {
    let policy = descriptor_for(call, registry, provider)
        .map(|descriptor| ResponsesPolicy { descriptor })?;
    Ok(Value::String(
        policy.complete_url(call.api_base.as_deref(), &call.environment),
    ))
}

fn responses_headers(
    call: &Call,
    registry: &JSONProviderRegistry,
    provider: ProviderId,
) -> Result<Value, Box<HelperError>> {
    let policy = descriptor_for(call, registry, provider)
        .map(|descriptor| ResponsesPolicy { descriptor })?;
    let (explicit, _) = keys(call);
    let headers =
        policy.validate_environment(sorted_headers(call), explicit.as_ref(), &call.environment);
    serde_json::to_value(headers).map_err(|error| helper_error("ValueError", error.to_string()))
}

fn responses_transform(
    call: &Call,
    registry: &JSONProviderRegistry,
    provider: ProviderId,
    model: &str,
) -> Result<Value, Box<HelperError>> {
    let policy = descriptor_for(call, registry, provider)
        .map(|descriptor| ResponsesPolicy { descriptor })?;
    let body = policy
        .transform_request(
            model,
            &ResponsesInput::Text(call.input.clone()),
            &call.params,
            None,
        )
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    serde_json::to_value(body).map_err(|error| helper_error("ValueError", error.to_string()))
}

// create_responses_config_class caches by slug for the process, so the first descriptor's class answers both
fn responses_cache(
    call: &Call,
    slug: &str,
    provider: ProviderId,
) -> Result<Value, Box<HelperError>> {
    let (Some(first), Some(second)) = (call.first.clone(), call.second.clone()) else {
        return Err(helper_error(
            "ValueError",
            format!("the {slug} cache call needs two descriptors"),
        ));
    };
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

fn generic_responses_url(call: &Call) -> Result<Value, Box<HelperError>> {
    let settings = openai_like_settings(call)?;
    let url = OpenAiLikeResponsesPolicy { settings }
        .complete_url(call.api_base.as_deref())
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    Ok(Value::String(url))
}

fn generic_responses_headers(call: &Call) -> Result<Value, Box<HelperError>> {
    let settings = openai_like_settings(call)?;
    let (explicit, _) = keys(call);
    let headers = OpenAiLikeResponsesPolicy { settings }
        .validate_environment(sorted_headers(call), explicit.as_ref());
    serde_json::to_value(headers).map_err(|error| helper_error("ValueError", error.to_string()))
}
