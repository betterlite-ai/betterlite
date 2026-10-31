use std::path::Path;

use crate::OracleError;
use serde::Deserialize;
use serde::de::IgnoredAny;

#[derive(Deserialize)]
#[serde(untagged)]
enum ConfigInspection {
    Sections(ConfigSections),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
struct ConfigSections {
    #[serde(default)]
    general_settings: Option<ConfigSection<GeneralSettingsInspection>>,
    #[serde(default)]
    environment_variables: Option<ConfigSection<EnvironmentVariablesInspection>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ConfigSection<T> {
    Fields(T),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
struct GeneralSettingsInspection {
    #[serde(default)]
    litellm_license: ForbiddenSetting,
}

#[derive(Deserialize)]
struct EnvironmentVariablesInspection {
    #[serde(default)]
    #[serde(rename = "LITELLM_LICENSE")]
    litellm_license: ForbiddenSetting,
}

#[derive(Default, Deserialize)]
#[serde(untagged)]
enum ForbiddenSetting {
    #[serde(skip)]
    #[default]
    Missing,
    Present(IgnoredAny),
}

pub(super) fn requests_license(path: &Path, config: &str) -> Result<bool, OracleError> {
    let parsed: ConfigInspection =
        serde_saphyr::from_str(config).map_err(|source| OracleError::CaseParse {
            path: path.to_owned(),
            source: Box::new(source),
        })?;
    let sections = match parsed {
        ConfigInspection::Sections(sections) => sections,
        ConfigInspection::Other(_) => return Ok(false),
    };
    let general_settings_license = sections
        .general_settings
        .is_some_and(|section| match section {
            ConfigSection::Fields(settings) => {
                matches!(settings.litellm_license, ForbiddenSetting::Present(_))
            }
            ConfigSection::Other(_) => false,
        });
    let environment_variable_license =
        sections
            .environment_variables
            .is_some_and(|section| match section {
                ConfigSection::Fields(variables) => {
                    matches!(variables.litellm_license, ForbiddenSetting::Present(_))
                }
                ConfigSection::Other(_) => false,
            });
    Ok(general_settings_license || environment_variable_license)
}
