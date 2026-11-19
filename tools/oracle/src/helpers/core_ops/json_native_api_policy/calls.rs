use std::collections::BTreeMap;

use betterlite_core::json_messages_policy::{
    JsonMessagesPolicy, MessagesCredentials, generic_complete_url, generic_transform_request,
    generic_validate_environment,
};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{
    JSONProviderRegistry, ProviderDescriptor as Descriptor,
};
use betterlite_openai::json_provider_policy::responses::{
    OpenAiLikeResponsesPolicy, ResponsesPolicy, ResponsesPolicyCache,
};
use betterlite_openai::response_input::ResponsesInput;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};

use super::capabilities::NoCapabilities;
use super::cases::{Call, Kind};
use super::{
    descriptor, helper_error, openai_like_settings, provider_id, runtime, sorted_headers,
    split_model,
};

// every case call answers one bounded question, so each handler turns the policy's own error into the case's error
type Helper<T> = Result<T, Box<HelperError>>;

pub(super) fn run_call(call: &Call, registry: &JSONProviderRegistry) -> Helper<Value> {
    let (slug, model) = split_model(&call.model)?;
    let provider = provider_id(&slug)?;
    match call.kind {
        Kind::GenericMessagesUrl => generic_messages_url(call),
        Kind::GenericMessagesHeaders => generic_messages_headers(call),
        Kind::GenericMessagesTransform => generic_messages_transform(call, &model),
        Kind::JsonMessagesUrl => json_messages_url(call, descriptor(call, registry, provider)?),
        Kind::JsonMessagesHeaders => {
            json_messages_headers(call, descriptor(call, registry, provider)?)
        }
        Kind::JsonMessagesTransform => {
            json_messages_transform(call, descriptor(call, registry, provider)?, &model)
        }
        Kind::ResponsesUrl => responses_url(call, descriptor(call, registry, provider)?),
        Kind::ResponsesHeaders => responses_headers(call, descriptor(call, registry, provider)?),
        Kind::ResponsesTransform => {
            responses_transform(call, descriptor(call, registry, provider)?, &model)
        }
        Kind::ResponsesCache => responses_cache(call, &slug, provider),
        Kind::GenericResponsesUrl => openai_like_url(call),
        Kind::GenericResponsesHeaders => openai_like_headers(call),
    }
}

fn call_credentials(call: &Call) -> MessagesCredentials<'_> {
    MessagesCredentials {
        api_base: call.api_base.as_deref(),
        api_key: call.api_key.as_ref(),
        global_api_key: call.global_api_key.as_ref(),
        lookup: &call.environment,
    }
}

fn generic_messages_url(call: &Call) -> Helper<Value> {
    generic_complete_url(call.api_base.as_deref())
        .or_value_error()
        .map(Value::String)
}

fn generic_messages_headers(call: &Call) -> Helper<Value> {
    let headers = generic_validate_environment(
        call.headers.clone(),
        call.api_key.as_ref(),
        &call.optional_params,
        &call.messages,
    )
    .or_value_error()?;
    serde_json::to_value(headers).or_value_error()
}

fn generic_messages_transform(call: &Call, model: &str) -> Helper<Value> {
    let mut capabilities = NoCapabilities;
    let messages = call.messages.clone();
    let params = call.optional_params.clone();
    let future = generic_transform_request(&mut capabilities, model, messages, params);
    serde_json::to_value(runtime()?.block_on(future).or_value_error()?).or_value_error()
}

fn json_messages_url(call: &Call, descriptor: Descriptor) -> Helper<Value> {
    JsonMessagesPolicy { descriptor }
        .complete_url(&call_credentials(call))
        .or_value_error()
        .map(Value::String)
}

fn json_messages_headers(call: &Call, descriptor: Descriptor) -> Helper<Value> {
    let policy = JsonMessagesPolicy { descriptor };
    let credentials = call_credentials(call);
    let params = &call.optional_params;
    let messages = &call.messages;
    let headers = policy
        .validate_environment(call.headers.clone(), &credentials, params, messages)
        .or_value_error()?;
    serde_json::to_value(headers).or_value_error()
}

fn json_messages_transform(call: &Call, descriptor: Descriptor, model: &str) -> Helper<Value> {
    let policy = JsonMessagesPolicy { descriptor };
    let mut capabilities = NoCapabilities;
    let future = policy.transform_request(
        &mut capabilities,
        model,
        call.messages.clone(),
        call.optional_params.clone(),
    );
    let body = runtime()?.block_on(future).or_value_error()?;
    serde_json::to_value(body).or_value_error()
}

fn responses_url(call: &Call, descriptor: Descriptor) -> Helper<Value> {
    let policy = ResponsesPolicy { descriptor };
    let url = policy.complete_url(call.api_base.as_deref(), &call.environment);
    serde_json::to_value(url).or_value_error()
}

fn responses_headers(call: &Call, descriptor: Descriptor) -> Helper<Value> {
    let policy = ResponsesPolicy { descriptor };
    let headers = policy.validate_environment(
        sorted_headers(call),
        call.api_key.as_ref(),
        &call.environment,
    );
    serde_json::to_value(headers).or_value_error()
}

fn responses_transform(call: &Call, descriptor: Descriptor, model: &str) -> Helper<Value> {
    let policy = ResponsesPolicy { descriptor };
    let input = ResponsesInput::Text(call.input.clone());
    let body = policy.transform_request(model, &input, &call.params, None);
    serde_json::to_value(body.or_value_error()?).or_value_error()
}

// create_responses_config_class caches by slug for the process, so the first descriptor's class answers both
fn responses_cache(call: &Call, slug: &str, provider: ProviderId) -> Helper<Value> {
    let (Some(first), Some(second)) = (call.first.clone(), call.second.clone()) else {
        return Err(helper_error(
            "ValueError",
            format!("the {slug} cache call needs two descriptors"),
        ));
    };
    let lookup = BTreeMap::<String, String>::new();
    let mut cache = ResponsesPolicyCache::default();
    let first_url = cache
        .policy_for_descriptor(Descriptor {
            provider,
            config: first,
        })
        .complete_url(None, &lookup);
    let second_url = cache
        .policy_for_descriptor(Descriptor {
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

fn openai_like_url(call: &Call) -> Helper<Value> {
    let settings = openai_like_settings(call)?;
    OpenAiLikeResponsesPolicy { settings }
        .complete_url(call.api_base.as_deref())
        .or_value_error()
        .map(Value::String)
}

fn openai_like_headers(call: &Call) -> Helper<Value> {
    let settings = openai_like_settings(call)?;
    let headers = OpenAiLikeResponsesPolicy { settings }
        .validate_environment(sorted_headers(call), call.api_key.as_ref());
    serde_json::to_value(headers).or_value_error()
}
