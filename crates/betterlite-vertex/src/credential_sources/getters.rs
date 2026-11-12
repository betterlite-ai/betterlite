use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

// the per-deployment litellm_params a Vertex call may carry, with the aliases LiteLLM reads
#[derive(Debug, Default)]
pub struct VertexParams<'params> {
    pub project: Option<&'params str>,
    pub ai_project: Option<&'params str>,
    pub location: Option<&'params str>,
    pub ai_location: Option<&'params str>,
    pub credentials: Option<&'params ConfiguredCredentials>,
    pub ai_credentials: Option<&'params ConfiguredCredentials>,
}

#[must_use]
pub fn safe_get_vertex_ai_project<'value>(
    params: &'value VertexParams<'_>,
    settings: &'value VertexSettings,
) -> Option<&'value str> {
    if let Some(project) = params.project {
        return Some(project);
    }
    if let Some(project) = params.ai_project {
        return Some(project);
    }
    settings.project.as_deref()
}

#[must_use]
pub fn safe_get_vertex_ai_credentials<'value>(
    params: &'value VertexParams<'_>,
    settings: &'value VertexSettings,
) -> Option<&'value ConfiguredCredentials> {
    if let Some(credentials) = params.credentials.or(params.ai_credentials) {
        return Some(credentials);
    }
    settings.credentials.as_ref()
}

// LiteLLM's explicit projection drops empty strings and never falls back to a module or env setting
#[must_use]
pub fn explicit_vertex_ai_location<'value>(
    params: &'value VertexParams<'_>,
) -> Option<&'value str> {
    params
        .location
        .filter(|location| !location.is_empty())
        .or_else(|| params.ai_location.filter(|location| !location.is_empty()))
}

#[must_use]
pub fn safe_get_vertex_ai_location<'value>(
    params: &'value VertexParams<'_>,
    settings: &'value VertexSettings,
) -> Option<&'value str> {
    if let Some(location) = explicit_vertex_ai_location(params) {
        return Some(location);
    }
    settings.location.as_deref()
}
