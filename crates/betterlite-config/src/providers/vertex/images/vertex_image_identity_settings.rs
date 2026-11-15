use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Sections;
use crate::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

// the environment an image call reads: the three vertex names and the two credential names
#[derive(Debug, Clone, Default)]
pub struct ImageIdentityEnvironment {
    pub project: Option<String>,
    pub location: Option<String>,
    pub legacy_location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
    pub application_credentials: Option<ConfiguredCredentials>,
}

// litellm_settings.vertex_project and litellm_settings.vertex_location, the litellm.vertex_* globals
#[derive(Debug, Clone, Default)]
pub struct ImageIdentityModule {
    pub project: Option<String>,
    pub location: Option<String>,
}

// what get_secret_str answers for the four names after the environment's own reads came back empty
#[derive(Debug, Clone, Default)]
pub struct ImageIdentitySecrets {
    pub project: Option<String>,
    pub location: Option<String>,
    pub legacy_location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
}

// LiteLLM constructs its image config classes empty, so the _vertex_* instance tier always loads empty
#[derive(Debug, Clone, Default)]
pub struct VertexImageIdentitySettings {
    pub environment: ImageIdentityEnvironment,
    pub module: ImageIdentityModule,
    pub secrets: ImageIdentitySecrets,
}

#[expect(
    clippy::multiple_inherent_impl,
    reason = "Each provider keeps its settings resolver beside its own module; google_pse holds the first impl"
)]
impl Sections {
    // the resolver reads the instance, environment and module; get_secret_str repeats the environment
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
        Ok(VertexImageIdentitySettings {
            environment: ImageIdentityEnvironment {
                project: present("VERTEXAI_PROJECT"),
                location: present("VERTEXAI_LOCATION"),
                legacy_location: present("VERTEX_LOCATION"),
                credentials: credentials.clone(),
                application_credentials,
            },
            module: ImageIdentityModule {
                project: present_setting(self.resource_sdk_settings.vertex_project.as_deref()),
                location: present_setting(self.resource_sdk_settings.vertex_location.as_deref()),
            },
            secrets: ImageIdentitySecrets {
                project: present("VERTEXAI_PROJECT"),
                location: present("VERTEXAI_LOCATION"),
                legacy_location: present("VERTEX_LOCATION"),
                credentials,
            },
        })
    }
}

fn present_setting(value: Option<&str>) -> Option<String> {
    value.filter(|text| !text.is_empty()).map(str::to_owned)
}

mod tests;
