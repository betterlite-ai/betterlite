use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

// the per-deployment litellm_params a Vertex call may carry, with the aliases LiteLLM reads
#[derive(Debug, Default, Clone, Copy)]
pub struct VertexParams<'params> {
    pub project: Option<&'params str>,
    pub ai_project: Option<&'params str>,
    pub location: Option<&'params str>,
    pub ai_location: Option<&'params str>,
    pub credentials: Option<&'params ConfiguredCredentials>,
    pub ai_credentials: Option<&'params ConfiguredCredentials>,
}

// LiteLLM's safe getters read both spellings, then the loaded setting, and change nothing they read
impl<'src> VertexParams<'src> {
    #[must_use]
    pub fn safe_get_vertex_ai_project(self, settings: &'src VertexSettings) -> Option<&'src str> {
        self.project
            .or(self.ai_project)
            .or(settings.project.as_deref())
    }

    #[must_use]
    pub fn safe_get_vertex_ai_credentials(
        self,
        settings: &'src VertexSettings,
    ) -> Option<&'src ConfiguredCredentials> {
        self.credentials
            .or(self.ai_credentials)
            .or(settings.credentials.as_ref())
    }

    // the explicit projection drops empty strings and never falls back to a module or env setting
    #[must_use]
    pub fn explicit_vertex_ai_location(self) -> Option<&'src str> {
        self.location
            .filter(|location| !location.is_empty())
            .or_else(|| self.ai_location.filter(|location| !location.is_empty()))
    }

    #[must_use]
    pub fn safe_get_vertex_ai_location(self, settings: &'src VertexSettings) -> Option<&'src str> {
        self.explicit_vertex_ai_location()
            .or(settings.location.as_deref())
    }
}
