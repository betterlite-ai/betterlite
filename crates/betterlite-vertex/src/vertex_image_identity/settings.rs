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

// the first tier holding text, since LiteLLM's `or` chains skip an empty string
fn first_non_empty<'tier>(tiers: &[Option<&'tier str>]) -> Option<&'tier str> {
    tiers
        .iter()
        .flatten()
        .copied()
        .find(|value| !value.is_empty())
}

// the winner of each `or` chain: LiteLLM skips an empty string, so it falls through to the next tier
#[derive(Debug, Clone, Copy)]
pub struct ResolvedIdentity<'settings> {
    pub project: Option<&'settings str>,
    pub location: Option<&'settings str>,
    pub credentials: Option<&'settings ConfiguredCredentials>,
}

impl ImageIdentitySettings {
    // the safe getters read the module and secret tiers; the resolver repeats them after the instance
    #[must_use]
    pub fn resolve<'settings>(
        &'settings self,
        params: VertexParams<'settings>,
    ) -> ResolvedIdentity<'settings> {
        let project = first_non_empty(&[
            params.project,
            params.ai_project,
            self.module.project.as_deref(),
            self.secrets.project.as_deref(),
            self.instance.project.as_deref(),
            self.environment.project.as_deref(),
        ]);
        // the explicit location params drop empty strings; then the module, both secrets, instance and environments
        let location = first_non_empty(&[
            params.explicit_vertex_ai_location(),
            self.module.location.as_deref(),
            self.secrets.location.as_deref(),
            self.secrets.legacy_location.as_deref(),
            self.instance.location.as_deref(),
            self.environment.location.as_deref(),
            self.environment.legacy_location.as_deref(),
        ]);
        // litellm.vertex_credentials sits after the environment spelling and before GOOGLE_APPLICATION_CREDENTIALS
        let credentials = [
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
        .find(|credentials| credentials.as_text().is_none_or(|text| !text.is_empty()));
        ResolvedIdentity {
            project,
            location,
            credentials,
        }
    }
}
