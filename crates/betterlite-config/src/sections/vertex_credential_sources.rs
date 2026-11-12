pub mod credential_source;
pub mod vertex_settings;

use std::path::PathBuf;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use credential_source::ConfiguredCredentials;
use vertex_settings::AdcEnvironment;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawSettings {
    vertex_project: Option<String>,
    vertex_location: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub project: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
    pub adc: AdcEnvironment,
}

pub type VertexSettings = Settings;

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            project: configured_project(self.litellm_settings.vertex_project, lookup)?,
            location: configured_location(self.litellm_settings.vertex_location, lookup)?,
            credentials: configured_credentials(lookup)?,
            adc: adc_environment(lookup),
        })
    }
}

fn configured_project<L>(
    configured: Option<String>,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    match configured {
        Some(text) => Ok(Some(resolve_string(
            text,
            "litellm_settings.vertex_project",
            lookup,
        )?)),
        None => Ok(lookup
            .lookup("VERTEXAI_PROJECT")
            .filter(|text| !text.is_empty())),
    }
}

fn configured_location<L>(
    configured: Option<String>,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    match configured {
        Some(text) => Ok(Some(resolve_string(
            text,
            "litellm_settings.vertex_location",
            lookup,
        )?)),
        None => Ok(lookup
            .lookup("VERTEXAI_LOCATION")
            .filter(|text| !text.is_empty())
            .or_else(|| {
                lookup
                    .lookup("VERTEX_LOCATION")
                    .filter(|text| !text.is_empty())
            })),
    }
}

// LiteLLM reads an empty environment variable as unset everywhere, unlike its last-alias get_secret_str
fn configured_credentials<L>(lookup: &L) -> Result<Option<ConfiguredCredentials>, ConfigError>
where
    L: EnvironmentLookup,
{
    lookup
        .lookup("VERTEXAI_CREDENTIALS")
        .filter(|text| !text.is_empty())
        .map_or_else(
            || Ok(None),
            |text| {
                ConfiguredCredentials::from_text(text)
                    .map(Some)
                    .map_err(|_source| ConfigError::InvalidScalar {
                        path: "vertex_credentials".to_owned(),
                        value: false,
                    })
            },
        )
}

fn adc_environment<L>(lookup: &L) -> AdcEnvironment
where
    L: EnvironmentLookup,
{
    let read = |name: &str| lookup.lookup(name).filter(|text| !text.is_empty());
    let gcloud = read("CLOUDSDK_CONFIG").map(PathBuf::from).or_else(|| {
        lookup
            .system_home()
            .map(|home| home.join(".config").join("gcloud"))
    });
    AdcEnvironment {
        application_credentials: read("GOOGLE_APPLICATION_CREDENTIALS").map(PathBuf::from),
        project: read("GOOGLE_CLOUD_PROJECT").or_else(|| read("GCLOUD_PROJECT")),
        gcloud_project: read("CLOUDSDK_CORE_PROJECT"),
        gcloud_credentials: gcloud
            .as_ref()
            .map(|dir| dir.join("application_default_credentials.json")),
        gcloud_configuration: gcloud
            .as_ref()
            .map(|dir| dir.join("configurations").join("config_default")),
        metadata_host: read("GCE_METADATA_HOST").or_else(|| read("GCE_METADATA_ROOT")),
    }
}
