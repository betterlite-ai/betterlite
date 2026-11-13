use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_predicates::params::ParamSupport;
use crate::static_eligibility::filter::Cause;

const RESPONSE_FORMAT: &str = "response_format";

fn text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) if !text.is_empty() => Some(text.as_str()),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}

// _deployment_model: base_model from model_info, else from the params, else the deployment's own model; an empty base_model falls to the model.
pub(crate) fn declared_model(spec: &DeploymentSpec) -> Option<&str> {
    let base = match spec.model_info.get("base_model") {
        None | Some(Json::Null) => spec.litellm_params.get("base_model"),
        other => other,
    };
    text(base).or_else(|| text(spec.litellm_params.get("model")))
}

// The response_format half of _pre_call_checks: a deployment whose provider does not support the param is not a candidate.
pub fn filter_supported_params<S>(
    deployments: Vec<DeploymentSpec>,
    response_format: bool,
    drop_params: bool,
    support: &S,
    causes: &mut Vec<Cause>,
) -> Vec<DeploymentSpec>
where
    S: ParamSupport + ?Sized,
{
    if drop_params || !response_format {
        return deployments;
    }
    let mut kept = Vec::new();
    for deployment in deployments {
        let Some(model) = declared_model(&deployment) else {
            kept.push(deployment);
            continue;
        };
        let provider = text(deployment.litellm_params.get("custom_llm_provider"));
        let supported = support
            .resolve_provider(model, provider)
            .and_then(|resolved| {
                support.supported_openai_params(
                    &resolved.model,
                    &resolved.custom_llm_provider,
                    None,
                )
            });
        match supported {
            Some(params) if !params.iter().any(|name| name == RESPONSE_FORMAT) => {
                causes.push(Cause::UnsupportedParameter {
                    model_name: deployment.model_name.clone(),
                    param: RESPONSE_FORMAT.to_owned(),
                });
            }
            Some(_) | None => kept.push(deployment),
        }
    }
    kept
}
