use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use crate::cooldown_decisions::decision::DeploymentFacts;
use crate::cooldown_decisions::failure::{ErrorStatus, Seconds};
use crate::cooldown_decisions::policy::AllowedFailsPolicy;
use crate::cooldown_policy::alternatives::{
    routing_group_has_alternatives, team_model_has_alternatives,
};
use crate::cooldown_policy::error::CooldownPolicyError;
use crate::cooldown_policy::event::CooldownEvent;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::lookup::CatalogLookup;

// get_api_base's provider default for a deployment that names no api_base
const OPENAI_API_BASE: &str = "https://api.openai.com";

// the facts _should_cooldown_deployment reads off its router instance, in the decision type B369 already declares
pub(crate) fn deployment_facts(
    lookup: &CatalogLookup<'_>,
    deployment: &DeploymentSpec,
    deployment_id: &str,
    requested_model_group: Option<&str>,
) -> Result<DeploymentFacts, CooldownPolicyError> {
    let model_group_size = lookup
        .get_model_list(Some(&deployment.model_name), None)?
        .len();
    Ok(DeploymentFacts {
        id: deployment_id.to_owned(),
        model_group_size: u32::try_from(model_group_size).unwrap_or(u32::MAX),
        has_alternatives: routing_group_has_alternatives(lookup, requested_model_group)
            || team_model_has_alternatives(lookup, deployment_id),
        provider_default: lookup
            .catalog
            .provider_default_ids
            .iter()
            .any(|candidate| candidate == deployment_id),
        allowed_fails_policy: allowed_fails_policy(deployment, deployment_id)?,
        allowed_fails: number_fact(&deployment.model_info, "allowed_fails", deployment_id)?,
        model_info_cooldown_time: number_fact(
            &deployment.model_info,
            "cooldown_time",
            deployment_id,
        )?,
        litellm_params_cooldown_time: number_fact(
            &deployment.litellm_params,
            "cooldown_time",
            deployment_id,
        )?,
    })
}

// deployment_callback_on_failure: deployment config first, then the header, then the router default, and a negative earlier value falls through
pub(crate) fn resolve_time_to_cooldown(
    deployment: &DeploymentSpec,
    header_retry_after: Option<Seconds>,
    router_default: Seconds,
) -> Result<Seconds, CooldownPolicyError> {
    let model_id = deployment.id().unwrap_or_default();
    let configured: Option<Seconds> =
        number_fact(&deployment.model_info, "cooldown_time", model_id)?.or(number_fact(
            &deployment.litellm_params,
            "cooldown_time",
            model_id,
        )?);
    if let Some(time) = configured.filter(|value| value.0 >= 0.0_f64) {
        return Ok(time);
    }
    Ok(header_retry_after
        .filter(|value| value.0 >= 0.0_f64)
        .unwrap_or(router_default))
}

// cast_exception_status_to_int, which the stored record and the fallback's 404 guard both read
pub(crate) fn error_status_code(status: &ErrorStatus) -> i64 {
    match status {
        ErrorStatus::Code(code) => *code,
        ErrorStatus::Text(text) => text.trim().parse().unwrap_or(500),
    }
}

// router_cooldown_event_callback builds the values from the deployment and passes exception_status on untouched
pub(crate) fn cooldown_event<'deployment>(
    deployment: &'deployment DeploymentSpec,
    deployment_id: &'deployment str,
    status_text: &'deployment str,
) -> CooldownEvent<'deployment> {
    let provider = provider(deployment);
    CooldownEvent {
        model_name: &deployment.model_name,
        model_id: deployment_id,
        api_base: api_base(deployment, provider),
        provider,
        exception_status: status_text,
    }
}

pub(crate) fn error_status_text(status: &ErrorStatus) -> String {
    match status {
        ErrorStatus::Code(code) => code.to_string(),
        ErrorStatus::Text(text) => text.clone(),
    }
}

// get_llm_provider's answer for the declaration the deployment carries: the custom provider, else the model prefix
fn provider(deployment: &DeploymentSpec) -> &str {
    if let Some(declared) = params_text(&deployment.litellm_params, "custom_llm_provider") {
        return declared;
    }
    let Some(model) = params_text(&deployment.litellm_params, "model") else {
        return "";
    };
    model.split_once('/').map_or("", |(prefix, _rest)| prefix)
}

// get_api_base over the fields a router deployment declares: api_base, the base_url alias, then the provider default
fn api_base<'deployment>(
    deployment: &'deployment DeploymentSpec,
    provider: &str,
) -> &'deployment str {
    if let Some(declared) = params_text(&deployment.litellm_params, "api_base") {
        return declared;
    }
    if let Some(alias) =
        params_text(&deployment.litellm_params, "base_url").filter(|value| !value.is_empty())
    {
        return alias;
    }
    if provider == "openai" {
        return OPENAI_API_BASE;
    }
    ""
}

// litellm_params.model as get_llm_provider and get_api_base read it
fn params_text<'spec>(params: &'spec JsonObject, key: &str) -> Option<&'spec str> {
    if let Some(Json::Text(value)) = params.get(key) {
        return Some(value.as_str());
    }
    None
}

// _get_deployment_cooldown_policy: anything that is not a mapping reads as no deployment policy
fn allowed_fails_policy(
    deployment: &DeploymentSpec,
    model_id: &str,
) -> Result<Option<AllowedFailsPolicy>, CooldownPolicyError> {
    let Some(Json::Object(policy)) = deployment.model_info.get("allowed_fails_policy") else {
        return Ok(None);
    };
    Ok(Some(AllowedFailsPolicy {
        content_policy_violation_error: number_fact(
            policy,
            "ContentPolicyViolationErrorAllowedFails",
            model_id,
        )?,
        bad_request_error: number_fact(policy, "BadRequestErrorAllowedFails", model_id)?,
        authentication_error: number_fact(policy, "AuthenticationErrorAllowedFails", model_id)?,
        timeout_error: number_fact(policy, "TimeoutErrorAllowedFails", model_id)?,
        rate_limit_error: number_fact(policy, "RateLimitErrorAllowedFails", model_id)?,
        internal_server_error: number_fact(policy, "InternalServerErrorAllowedFails", model_id)?,
        service_unavailable_error: number_fact(
            policy,
            "ServiceUnavailableErrorAllowedFails",
            model_id,
        )?,
        bad_gateway_error: number_fact(policy, "BadGatewayErrorAllowedFails", model_id)?,
        not_found_error: number_fact(policy, "NotFoundErrorAllowedFails", model_id)?,
    }))
}

trait FactValue: Sized {
    fn read(number: &Number) -> Option<Self>;
}

impl FactValue for u32 {
    fn read(number: &Number) -> Option<Self> {
        let count = number.as_u64()?;
        Self::try_from(count).ok()
    }
}

impl FactValue for Seconds {
    fn read(number: &Number) -> Option<Self> {
        Some(Self(number.as_f64().unwrap_or(0.0_f64)))
    }
}

// a numeric fact's value or the type error that names the deployment and field
fn number_fact<T>(
    info: &JsonObject,
    field: &'static str,
    model_id: &str,
) -> Result<Option<T>, CooldownPolicyError>
where
    T: FactValue,
{
    match info.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => {
            T::read(number)
                .map(Some)
                .ok_or_else(|| CooldownPolicyError::DeploymentFact {
                    model_id: model_id.to_owned(),
                    field,
                })
        }
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(CooldownPolicyError::DeploymentFact {
                model_id: model_id.to_owned(),
                field,
            })
        }
    }
}
