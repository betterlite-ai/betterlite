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
            project: resolve_setting(
                self.litellm_settings.vertex_project,
                "litellm_settings.vertex_project",
                &["VERTEXAI_PROJECT"],
                lookup,
            )?,
            location: resolve_setting(
                self.litellm_settings.vertex_location,
                "litellm_settings.vertex_location",
                &["VERTEXAI_LOCATION", "VERTEX_LOCATION"],
                lookup,
            )?,
            credentials: parse_credentials(
                lookup
                    .lookup("VERTEXAI_CREDENTIALS")
                    .filter(|text| !text.is_empty()),
            )?,
            adc: adc_environment(lookup),
        })
    }
}

fn resolve_setting<L>(
    configured: Option<String>,
    path: &str,
    environment: &[&str],
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    if let Some(text) = configured {
        return resolve_string(text, path, lookup).map(Some);
    }
    Ok(environment
        .iter()
        .find_map(|name| lookup.lookup(name).filter(|text| !text.is_empty())))
}

// LiteLLM reads an empty environment variable as unset everywhere, unlike its last-alias get_secret_str
fn parse_credentials(text: Option<String>) -> Result<Option<ConfiguredCredentials>, ConfigError> {
    let Some(credentials) = text else {
        return Ok(None);
    };
    ConfiguredCredentials::from_text(credentials)
        .map(Some)
        .map_err(|_source| ConfigError::InvalidScalar {
            path: "vertex_credentials".to_owned(),
            value: false,
        })
}

fn adc_environment(lookup: &dyn EnvironmentLookup) -> AdcEnvironment {
    let read = |name: &str| lookup.lookup(name).filter(|text| !text.is_empty());
    let gcloud = read("CLOUDSDK_CONFIG").map(PathBuf::from).or_else(|| {
        lookup
            .system_home()
            .map(|home| home.join(".config").join("gcloud"))
    });
    let active = read("CLOUDSDK_ACTIVE_CONFIG_NAME").unwrap_or_else(|| "default".to_owned());
    AdcEnvironment {
        application_credentials: read("GOOGLE_APPLICATION_CREDENTIALS").map(PathBuf::from),
        project: read("GOOGLE_CLOUD_PROJECT").or_else(|| read("GCLOUD_PROJECT")),
        gcloud_project: read("CLOUDSDK_CORE_PROJECT"),
        gcloud_credentials: gcloud
            .as_ref()
            .map(|dir| dir.join("application_default_credentials.json")),
        gcloud_configuration: gcloud
            .as_ref()
            .map(|dir| dir.join("configurations").join(format!("config_{active}"))),
        metadata_host: read("GCE_METADATA_HOST").or_else(|| read("GCE_METADATA_ROOT")),
    }
}
