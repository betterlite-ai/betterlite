use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

use crate::credential_sources::getters::VertexParams;

// LiteLLM reads getattr(self, "_vertex_project", None) and friends before anything else
#[derive(Debug, Clone, Default)]
pub struct ImageInstance {
    pub project: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
}

// The os.environ values an image call reads; the loader drops empty variables, as everywhere else
#[derive(Debug, Clone, Default)]
pub struct ImageEnvironment {
    pub project: Option<String>,
    pub location: Option<String>,
    pub legacy_location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
    pub application_credentials: Option<ConfiguredCredentials>,
}

// litellm.vertex_project, litellm.vertex_location and litellm.vertex_credentials
#[derive(Debug, Clone, Default)]
pub struct ImageModule {
    pub project: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
}

// What get_secret_str answers for each name after every other source came back empty
#[derive(Debug, Clone, Default)]
pub struct ImageSecrets {
    pub project: Option<String>,
    pub location: Option<String>,
    pub legacy_location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
}

#[derive(Debug, Clone, Default)]
pub struct ImageIdentitySettings {
    pub instance: ImageInstance,
    pub environment: ImageEnvironment,
    pub module: ImageModule,
    pub secrets: ImageSecrets,
}

// every tier joins an `or` chain in LiteLLM, so an empty value falls through to the next one
impl ImageIdentitySettings {
    // the safe getter reads the module and secret tiers; the resolver repeats them after the instance
    #[must_use]
    pub fn resolve_project<'settings>(
        &'settings self,
        params: VertexParams<'settings>,
    ) -> Option<&'settings str> {
        [
            params.project,
            params.ai_project,
            self.module.project.as_deref(),
            self.secrets.project.as_deref(),
            self.instance.project.as_deref(),
            self.environment.project.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
    }

    // the explicit location params drop empty strings; then the module, both secrets, instance and environments
    #[must_use]
    pub fn resolve_location<'settings>(
        &'settings self,
        params: VertexParams<'settings>,
    ) -> Option<&'settings str> {
        [
            params.explicit_vertex_ai_location(),
            self.module.location.as_deref(),
            self.secrets.location.as_deref(),
            self.secrets.legacy_location.as_deref(),
            self.instance.location.as_deref(),
            self.environment.location.as_deref(),
            self.environment.legacy_location.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
    }

    // litellm.vertex_credentials sits after the environment spelling and before GOOGLE_APPLICATION_CREDENTIALS
    #[must_use]
    pub fn resolve_credentials<'settings>(
        &'settings self,
        params: VertexParams<'settings>,
    ) -> Option<&'settings ConfiguredCredentials> {
        [
            params.credentials,
            params.ai_credentials,
            self.secrets.credentials.as_ref(),
            self.instance.credentials.as_ref(),
            self.environment.credentials.as_ref(),
            self.module.credentials.as_ref(),
            self.environment.application_credentials.as_ref(),
        ]
        .into_iter()
        .flatten()
        .find(|credentials| credentials.as_text().is_none_or(|text| !text.is_empty()))
    }
}
