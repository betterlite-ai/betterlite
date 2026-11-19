use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::thinking::is_effort_used;
use betterlite_anthropic::anthropic_capability_catalog::{
    CatalogSources, DropParams, OptionalParams,
};
use betterlite_anthropic::capabilities::supported_params::get_supported_openai_params;
use betterlite_anthropic::params::error::ParamsError;
use betterlite_anthropic::params::{ParamsRequest, map_openai_params};
use betterlite_core::cost::completion::lookup::NoHfConfig;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::params::classification::RESERVED_PARAMETER_NAMES;
use betterlite_models::params::{
    CallType, ChatParameters, ParameterError, ParameterMapping, ParameterPolicy,
    map_listed_parameters,
};
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::anthropic_dispatch::gateway::runtime::AnthropicRuntime;
use crate::operation::{AnthropicCallError, OperationError};

// The runtime values every parameter step reads, borrowed once so each step takes one argument.
pub(crate) struct Sources<'context, 'catalog> {
    pub(crate) runtime: &'context AnthropicRuntime,
    pub(crate) environment:
        &'context mut CapabilityEnvironment<'catalog, NoDynamicMetadata, NoHfConfig>,
    pub(crate) catalogs: &'context CatalogSources<'catalog>,
    pub(crate) model: &'context str,
}

pub(crate) struct MappedParameters {
    pub(crate) optional: IndexMap<String, Json>,
    pub(crate) headers: BTreeMap<String, SecretString>,
    pub(crate) effort_used: bool,
}

// get_optional_params at the pin: the request's own kwargs become the parameter policy and the rest is checked and mapped.
pub(crate) async fn map(
    sources: &mut Sources<'_, '_>,
    params: IndexMap<String, JsonValue>,
    stream: bool,
) -> Result<MappedParameters, OperationError> {
    let mut fields = params;
    let headers = take_headers(&mut fields)?;
    let extra_body = fields.shift_remove("extra_body");
    let (policy, call_drop) = take_policy(&mut fields, sources.runtime.global_drop_params)?;
    for reserved in RESERVED_PARAMETER_NAMES {
        fields.shift_remove(*reserved);
    }
    if stream {
        fields.insert(String::from("stream"), JsonValue::Bool(true));
    }
    let parameters: ChatParameters = convert(JsonValue::Object(fields))?;
    let optional =
        optional_parameters(sources, &parameters, &policy, call_drop, extra_body).await?;
    let effort_used = effort(&optional, sources).await;
    Ok(MappedParameters {
        optional,
        headers,
        effort_used,
    })
}

async fn optional_parameters(
    sources: &mut Sources<'_, '_>,
    parameters: &ChatParameters,
    policy: &ParameterPolicy,
    call_drop: bool,
    extra_body: Option<JsonValue>,
) -> Result<IndexMap<String, Json>, OperationError> {
    let mapping = mapping(sources, parameters, policy).await?;
    let standard: IndexMap<String, Json> = convert(mapping.standard)?;
    let provider: IndexMap<String, Json> = convert(mapping.provider_fields)?;
    let request = ParamsRequest {
        model: sources.model,
        provider: "anthropic",
        drop: DropParams {
            global: sources.runtime.global_drop_params,
            call: call_drop,
        },
        budgets: &sources.runtime.budgets,
        default_max_tokens: sources.runtime.max_tokens,
        catalogs: sources.catalogs,
    };
    let mut optional = IndexMap::new();
    map_openai_params(
        &request,
        &standard,
        &mut optional,
        &mut *sources.environment,
    )
    .await
    .map_err(params_error)?;
    copy_provider_fields(provider, &mut optional);
    copy_extra_body(extra_body, policy, &mut optional)?;
    Ok(optional)
}

// _check_valid_arg: a name outside the supported list is UnsupportedParamsError unless drop_params takes it out.
async fn mapping(
    sources: &mut Sources<'_, '_>,
    parameters: &ChatParameters,
    policy: &ParameterPolicy,
) -> Result<ParameterMapping, OperationError> {
    let supported = get_supported_openai_params(
        sources.model,
        Some("anthropic"),
        &mut *sources.environment,
        sources.catalogs,
    )
    .await;
    map_listed_parameters(
        &supported,
        sources.model,
        CallType::ChatCompletion,
        parameters,
        policy,
    )
    .map_err(|source| OperationError::Parameters { source })
}

async fn effort(optional: &IndexMap<String, Json>, sources: &mut Sources<'_, '_>) -> bool {
    let params = OptionalParams {
        reasoning_effort: optional.get("reasoning_effort").cloned(),
        thinking: optional.get("thinking").cloned(),
        output_config: optional.get("output_config").cloned(),
    };
    is_effort_used(
        Some(&params),
        sources.model,
        "anthropic",
        &mut *sources.environment,
        sources.catalogs,
    )
    .await
}

fn take_headers(
    fields: &mut IndexMap<String, JsonValue>,
) -> Result<BTreeMap<String, SecretString>, OperationError> {
    match fields.shift_remove("extra_headers") {
        None | Some(JsonValue::Null) => Ok(BTreeMap::new()),
        Some(value) => convert(value),
    }
}

fn take_policy(
    fields: &mut IndexMap<String, JsonValue>,
    global: bool,
) -> Result<(ParameterPolicy, bool), OperationError> {
    let call_drop = match fields.shift_remove("drop_params") {
        None | Some(JsonValue::Null) => false,
        Some(value) => convert(value)?,
    };
    let policy = ParameterPolicy {
        drop_params: call_drop || global,
        additional_drop_params: names(fields.shift_remove("additional_drop_params"))?,
        allowed_openai_params: names(fields.shift_remove("allowed_openai_params"))?,
    };
    Ok((policy, call_drop))
}

fn names(value: Option<JsonValue>) -> Result<Vec<String>, OperationError> {
    match value {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(value) => convert(value),
    }
}

// Provider-specific params go into optional_params at the top level, with nulls skipped.
fn copy_provider_fields(fields: IndexMap<String, Json>, optional: &mut IndexMap<String, Json>) {
    for (name, value) in fields {
        if value == Json::Null {
            continue;
        }
        optional.insert(name, value);
    }
}

fn copy_extra_body(
    extra_body: Option<JsonValue>,
    policy: &ParameterPolicy,
    optional: &mut IndexMap<String, Json>,
) -> Result<(), OperationError> {
    let Some(value) = extra_body.filter(|value| *value != JsonValue::Null) else {
        return Ok(());
    };
    if policy
        .additional_drop_params
        .iter()
        .any(|name| name == "extra_body")
    {
        return Ok(());
    }
    optional.insert(String::from("extra_body"), convert(value)?);
    Ok(())
}

fn convert<T, U>(value: T) -> Result<U, OperationError>
where
    T: Serialize,
    U: DeserializeOwned,
{
    let wire = serde_json::to_value(value).map_err(|source| OperationError::Encode { source })?;
    serde_json::from_value(wire).map_err(|source| OperationError::Parameters {
        source: ParameterError::Encode { source },
    })
}

// A mapped parameter that LiteLLM rejects keeps the status it raised, so the caller reads it as the provider's own error.
fn params_error(source: ParamsError) -> OperationError {
    match source.status_code() {
        Some(status) => OperationError::Provider {
            response: Box::new(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: source.to_string().into_bytes(),
            }),
        },
        None => OperationError::Anthropic {
            source: Box::new(AnthropicCallError::Anthropic {
                status_code: 500,
                message: source.to_string(),
                headers: None,
            }),
        },
    }
}
