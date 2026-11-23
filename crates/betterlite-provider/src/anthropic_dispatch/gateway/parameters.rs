use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::thinking::is_effort_used;
use betterlite_anthropic::anthropic_capability_catalog::{
    CatalogSources, DropParams, OptionalParams,
};
use betterlite_anthropic::capabilities::supported_params::get_supported_openai_params;
use betterlite_anthropic::params::error::ParamsError;
use betterlite_anthropic::params::{ParamsRequest, map_openai_params};
use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::filters::apply_openai_param_overrides;
use betterlite_core::cost::completion::lookup::NoHfConfig;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::params::{ParameterMapping, ParameterPolicy};
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::anthropic_dispatch::gateway::runtime::AnthropicRuntime;
use crate::operation::{AnthropicCallError, OperationError};
use crate::prepared::body::{convert, drop_nested_paths, listed_parameters, parameter_policy};
use crate::prepared::deployment::Deployment;

// The runtime values every parameter step reads, borrowed once so each step takes one argument.
pub(crate) struct Sources<'context, 'catalog> {
    pub(crate) runtime: &'context AnthropicRuntime,
    pub(crate) deployment: &'context Deployment,
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
    let (policy, call_drop) = parameter_policy(&fields, sources.deployment)?;
    if stream {
        fields.insert(String::from("stream"), JsonValue::Bool(true));
    }
    let supported = get_supported_openai_params(
        sources.model,
        Some("anthropic"),
        &mut *sources.environment,
        sources.catalogs,
    )
    .await;
    let mapping = listed_parameters(&fields, &supported, sources.model, sources.deployment)?;
    let optional = optional_parameters(sources, mapping, &policy, call_drop, extra_body).await?;
    let effort_used = effort(&optional, sources).await;
    Ok(MappedParameters {
        optional,
        headers,
        effort_used,
    })
}

async fn optional_parameters(
    sources: &mut Sources<'_, '_>,
    mapping: ParameterMapping,
    policy: &ParameterPolicy,
    call_drop: bool,
    extra_body: Option<JsonValue>,
) -> Result<IndexMap<String, Json>, OperationError> {
    let standard: IndexMap<String, Json> = convert(mapping.standard)?;
    let provider: IndexMap<String, Json> = convert(mapping.provider_fields)?;
    let request = ParamsRequest {
        model: sources.model,
        provider: "anthropic",
        drop: DropParams {
            global: sources.deployment.global_drop_params,
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
    .map_err(|source| params_error(&source))?;
    copy_provider_fields(provider, &mut optional);
    optional = openai_overrides(optional, standard, policy)?;
    copy_extra_body(extra_body, policy, &mut optional)?;
    drop_nested_paths(&mut optional, &policy.additional_drop_params)?;
    Ok(optional)
}

// _apply_openai_param_overrides: a name the caller allowed reaches the body as sent, even when the provider mapper does not map it.
fn openai_overrides(
    optional: IndexMap<String, Json>,
    standard: IndexMap<String, Json>,
    policy: &ParameterPolicy,
) -> Result<IndexMap<String, Json>, OperationError> {
    let mapped: Params = convert(optional)?;
    let mut non_default: Params = convert(standard)?;
    let merged =
        apply_openai_param_overrides(mapped, &mut non_default, &policy.allowed_openai_params);
    convert(merged)
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

// A mapped parameter that LiteLLM rejects keeps the status it raised, so the caller reads it as the provider's own error.
fn params_error(source: &ParamsError) -> OperationError {
    source.status_code().map_or_else(
        || OperationError::Anthropic {
            source: Box::new(AnthropicCallError::Anthropic {
                status_code: 500,
                message: source.to_string(),
                headers: None,
            }),
        },
        |status| OperationError::Provider {
            response: Box::new(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: source.to_string().into_bytes(),
            }),
        },
    )
}
