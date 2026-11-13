use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::static_eligibility::filter::Cause;

// _deployment_supports_web_search reads the deployment's own override; with none set every deployment supports web search.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WebSearchError {
    #[error("model_info.supports_web_search must be a boolean")]
    NotBoolean,
}

fn is_web_search_tool(tool: &Json) -> bool {
    match tool {
        Json::Object(fields) => matches!(
            fields.get("type"),
            Some(Json::Text(kind)) if kind == "web_search" || kind == "web_search_preview"
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    }
}

fn supports_web_search(spec: &DeploymentSpec) -> Result<bool, WebSearchError> {
    match spec.model_info.get("supports_web_search") {
        None | Some(Json::Null) => Ok(true),
        Some(Json::Bool(supported)) => Ok(*supported),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(WebSearchError::NotBoolean)
        }
    }
}

// filter_web_search_deployments: a web search request keeps only the deployments that declare support for it.
pub fn filter_web_search(
    deployments: Vec<DeploymentSpec>,
    tools: &[Json],
    causes: &mut Vec<Cause>,
) -> Result<Vec<DeploymentSpec>, WebSearchError> {
    if !tools.iter().any(is_web_search_tool) {
        return Ok(deployments);
    }
    let mut kept = Vec::new();
    for deployment in deployments {
        if supports_web_search(&deployment)? {
            kept.push(deployment);
        } else {
            causes.push(Cause::WebSearch {
                model_name: deployment.model_name.clone(),
            });
        }
    }
    Ok(kept)
}
