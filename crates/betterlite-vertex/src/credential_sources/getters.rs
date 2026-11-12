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
    params
        .project
        .or(params.ai_project)
        .or(settings.project.as_deref())
}

#[must_use]
pub fn safe_get_vertex_ai_credentials<'value>(
    params: &'value VertexParams<'_>,
    settings: &'value VertexSettings,
) -> Option<&'value ConfiguredCredentials> {
    params
        .credentials
        .or(params.ai_credentials)
        .or(settings.credentials.as_ref())
}

// LiteLLM's explicit projection drops empty strings and never falls back to a module or env setting
#[must_use]
pub fn explicit_vertex_ai_location<'value>(
    params: &'value VertexParams<'_>,
) -> Option<&'value str> {
    params
        .location
        .filter(|location| !location.is_empty())
        .or(params.ai_location.filter(|location| !location.is_empty()))
}

#[must_use]
pub fn safe_get_vertex_ai_location<'value>(
    params: &'value VertexParams<'_>,
    settings: &'value VertexSettings,
) -> Option<&'value str> {
    match explicit_vertex_ai_location(params) {
        Some(location) => Some(location),
        None => settings.location.as_deref(),
    }
}
