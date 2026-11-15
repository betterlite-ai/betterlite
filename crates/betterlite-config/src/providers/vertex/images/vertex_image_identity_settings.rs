use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Sections;
use crate::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

// the environment an image call reads: VERTEXAI_PROJECT, VERTEXAI_LOCATION, VERTEX_LOCATION,
// VERTEXAI_CREDENTIALS and GOOGLE_APPLICATION_CREDENTIALS
pub(crate) type ImageEnvironmentTier = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<ConfiguredCredentials>,
    Option<ConfiguredCredentials>,
);

// litellm_settings.vertex_project and litellm_settings.vertex_location, the litellm.vertex_* globals
pub(crate) type ImageModuleTier = (Option<String>, Option<String>);

// what get_secret_str answers for the four names after the environment's own reads came back empty
pub(crate) type ImageSecretTier = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<ConfiguredCredentials>,
);

// LiteLLM constructs its image config classes empty, so the _vertex_* instance tier always loads empty
pub(crate) type VertexImageIdentitySettings =
    (ImageEnvironmentTier, ImageModuleTier, ImageSecretTier);

#[expect(
    clippy::multiple_inherent_impl,
    reason = "Each provider keeps its settings resolver beside its own module; google_pse holds the first impl"
)]
impl Sections {
    // _resolve_vertex_project and friends read the instance override, the environment and the module
    // value, and get_secret_str falls back to the same environment a name was already read from
    pub fn vertex_image_identity_settings(
        &self,
        lookup: &dyn EnvironmentLookup,
    ) -> Result<VertexImageIdentitySettings, ConfigError> {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        let credentials = self.vertex_credential_sources.credentials.clone();
        let application_credentials = present("GOOGLE_APPLICATION_CREDENTIALS")
            .map(ConfiguredCredentials::from_text)
            .transpose()
            .map_err(|_source| ConfigError::InvalidScalar {
                path: "GOOGLE_APPLICATION_CREDENTIALS".to_owned(),
                value: false,
            })?;
        let environment = (
            present("VERTEXAI_PROJECT"),
            present("VERTEXAI_LOCATION"),
            present("VERTEX_LOCATION"),
            credentials.clone(),
            application_credentials,
        );
        let module = (
            present_setting(&self.resource_sdk_settings.vertex_project),
            present_setting(&self.resource_sdk_settings.vertex_location),
        );
        let secrets = (
            present("VERTEXAI_PROJECT"),
            present("VERTEXAI_LOCATION"),
            present("VERTEX_LOCATION"),
            credentials,
        );
        Ok((environment, module, secrets))
    }
}

fn present_setting(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

mod tests;
