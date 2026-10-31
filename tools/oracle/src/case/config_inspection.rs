use std::collections::BTreeMap;
use std::path::Path;

use crate::OracleError;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(untagged)]
enum ConfigInspection {
    Sections(ConfigSections),
    Null(()),
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Sequence(Vec<InspectionValue>),
}

#[derive(Deserialize)]
struct ConfigSections {
    #[serde(default)]
    general_settings: Option<GeneralSettingsInspection>,
    #[serde(default)]
    environment_variables: Option<EnvironmentVariablesInspection>,
}

#[derive(Deserialize)]
struct GeneralSettingsInspection {
    #[serde(default)]
    litellm_license: ForbiddenSetting,
    #[serde(flatten)]
    _other: BTreeMap<String, InspectionValue>,
}

#[derive(Deserialize)]
struct EnvironmentVariablesInspection {
    #[serde(default)]
    #[serde(rename = "LITELLM_LICENSE")]
    litellm_license: ForbiddenSetting,
    #[serde(flatten)]
    _other: BTreeMap<String, InspectionValue>,
}

#[derive(Default, Deserialize)]
#[serde(untagged)]
enum ForbiddenSetting {
    #[serde(skip)]
    #[default]
    Missing,
    Null(()),
    Value(InspectionValue),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum InspectionValue {
    Null(()),
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Sequence(Vec<Self>),
    Mapping(BTreeMap<String, Self>),
}

pub(super) fn requests_license(path: &Path, config: &str) -> Result<bool, OracleError> {
    let parsed: ConfigInspection =
        serde_saphyr::from_str(config).map_err(|source| OracleError::CaseParse {
            path: path.to_owned(),
            source: Box::new(source),
        })?;
    let sections = match parsed {
        ConfigInspection::Sections(sections) => sections,
        ConfigInspection::Null(value) => {
            let _: &() = &value;
            return Ok(false);
        }
        ConfigInspection::Boolean(value) => {
            let _: &bool = &value;
            return Ok(false);
        }
        ConfigInspection::Integer(value) => {
            let _: &i64 = &value;
            return Ok(false);
        }
        ConfigInspection::Float(value) => {
            let _: &f64 = &value;
            return Ok(false);
        }
        ConfigInspection::Text(value) => {
            let _: &String = &value;
            return Ok(false);
        }
        ConfigInspection::Sequence(values) => {
            let _: bool = values.iter().all(inspection_value_is_present);
            return Ok(false);
        }
    };
    let general_settings_license = sections
        .general_settings
        .is_some_and(|settings| forbidden_setting_is_present(&settings.litellm_license));
    let environment_variable_license = sections
        .environment_variables
        .is_some_and(|variables| forbidden_setting_is_present(&variables.litellm_license));
    Ok(general_settings_license || environment_variable_license)
}

fn forbidden_setting_is_present(setting: &ForbiddenSetting) -> bool {
    match setting {
        ForbiddenSetting::Missing => false,
        ForbiddenSetting::Null(()) => true,
        ForbiddenSetting::Value(value) => inspection_value_is_present(value),
    }
}

fn inspection_value_is_present(value: &InspectionValue) -> bool {
    match value {
        InspectionValue::Null(()) => true,
        InspectionValue::Boolean(boolean_value) => {
            let _: &bool = boolean_value;
            true
        }
        InspectionValue::Integer(integer_value) => {
            let _: &i64 = integer_value;
            true
        }
        InspectionValue::Float(float_value) => {
            let _: &f64 = float_value;
            true
        }
        InspectionValue::Text(text_value) => {
            let _: &String = text_value;
            true
        }
        InspectionValue::Sequence(values) => values.iter().all(inspection_value_is_present),
        InspectionValue::Mapping(values) => values.values().all(inspection_value_is_present),
    }
}
