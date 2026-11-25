use betterlite_models::model_registration::JsonObject;
use tracing::{debug, warn};

use super::non_empty_text;

// The one LiteLLM_ManagedVectorStore field _override_vertex_params_from_router_credentials reads
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RouterCredentials {
    pub litellm_params: Option<JsonObject>,
}

#[must_use]
pub fn override_vertex_params(
    router_credentials: Option<&RouterCredentials>,
    project: Option<String>,
    location: Option<String>,
) -> (Option<String>, Option<String>) {
    let Some(credentials) = router_credentials else {
        return (project, location);
    };
    debug!("Using vector store credentials to override vertex project and location");
    let Some(params) = credentials
        .litellm_params
        .as_ref()
        .filter(|params| !params.is_empty())
    else {
        warn!("Vector store credentials found but litellm_params is empty");
        return (project, location);
    };
    let vector_project = params.get("vertex_project").and_then(non_empty_text);
    let vector_location = params.get("vertex_location").and_then(non_empty_text);
    if let Some(value) = vector_project {
        debug!(
            url_project = ?project,
            vector_store_project = %value,
            "Overriding vertex_project from URL with vector store value"
        );
    } else {
        warn!("Vector store credentials found but missing vertex_project in litellm_params");
    }
    if let Some(value) = vector_location {
        debug!(
            url_location = ?location,
            vector_store_location = %value,
            "Overriding vertex_location from URL with vector store value"
        );
    } else {
        warn!("Vector store credentials found but missing vertex_location in litellm_params");
    }
    (
        vector_project.map(str::to_owned).or(project),
        vector_location.map(str::to_owned).or(location),
    )
}
