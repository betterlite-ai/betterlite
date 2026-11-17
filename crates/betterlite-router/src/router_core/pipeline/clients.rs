use betterlite_models::decisions::Json;

use crate::deployment_clients::clients::{CachedClient, ClientDeployment, ClientLookup};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::route::RouteContext;

// Router._get_client for the selected deployment: the client its provider call runs with
pub(super) fn acquire_client(
    context: &RouteContext<'_>,
    group: &str,
    deployment: &DeploymentSpec,
) -> Option<CachedClient> {
    let access = context.clients?;
    let model_id = deployment.id()?;
    let api_base = match deployment.litellm_params.get("api_base") {
        Some(Json::Text(base)) => base.as_str(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            ""
        }
    };
    Some(access.clients.openai_client(
        access.kind,
        &ClientLookup {
            model_id,
            stream: access.stream,
            api_key: None,
        },
        &ClientDeployment {
            model_group: group,
            api_base,
        },
    ))
}
