#[path = "team_admin_fields/tests.rs"]
mod tests;

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::JsonValue;

pub const EDITABLE_FIELDS_SETTING: &str = "team_admin_editable_team_fields";
pub const SUPPORTED_TEAM_FIELDS: [&str; 3] = ["tpm_limit", "rpm_limit", "max_budget"];
pub const PROJECTS_PERMISSION: &str = "projects";
pub const MEMBER_KEY_BUDGETS_PERMISSION: &str = "member_key_budgets";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneralSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneralSettings {
    #[serde(rename = "team_admin_editable_team_fields")]
    editable_team_fields: Option<JsonValue>,
}

// The setting stays unparsed: which names a proxy supports depends on the caller, so resolution warns at use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub editable_team_fields: Option<JsonValue>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            editable_team_fields: self.general_settings.editable_team_fields,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingWarning {
    NotAList {
        raw: JsonValue,
    },
    Unsupported {
        names: Vec<String>,
        supported: Vec<String>,
    },
}

impl SettingWarning {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NotAList { raw } => {
                format!("{EDITABLE_FIELDS_SETTING} must be a list of field names; ignoring {raw}")
            }
            Self::Unsupported { names, supported } => format!(
                "{EDITABLE_FIELDS_SETTING} ignores unsupported field(s) {}; supported: {}",
                names.join(", "),
                supported.join(", ")
            ),
        }
    }
}

// The names a setting grants out of `supported`, plus what it rejected so the caller can log or report it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolution {
    pub fields: BTreeSet<String>,
    pub warnings: Vec<SettingWarning>,
}

#[must_use]
pub fn resolve_editable_fields(value: Option<&JsonValue>, supported: &[&str]) -> Resolution {
    let Some(raw) = value.filter(|found| !found.is_null()) else {
        return Resolution::default();
    };
    let Some(configured) = configured_names(raw) else {
        let warning = SettingWarning::NotAList { raw: raw.clone() };
        tracing::warn!("{}", warning.message());
        return Resolution {
            fields: BTreeSet::new(),
            warnings: vec![warning],
        };
    };
    let unsupported = unsupported_names(&configured, supported);
    let mut warnings = Vec::new();
    if !unsupported.is_empty() {
        warnings.push(unsupported_warning(unsupported, supported));
    }
    let fields: BTreeSet<String> = configured
        .into_iter()
        .filter(|name| supported.contains(&name.as_str()))
        .collect();
    Resolution { fields, warnings }
}

fn configured_names(raw: &JsonValue) -> Option<BTreeSet<String>> {
    let JsonValue::Array(items) = raw else {
        return None;
    };
    items
        .iter()
        .map(|item| match item {
            JsonValue::String(name) => Some(name.clone()),
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Object(_)
            | JsonValue::Array(_) => None,
        })
        .collect()
}

fn unsupported_names(configured: &BTreeSet<String>, supported: &[&str]) -> Vec<String> {
    configured
        .iter()
        .filter(|name| {
            !supported.contains(&name.as_str())
                && !matches!(
                    name.as_str(),
                    PROJECTS_PERMISSION | MEMBER_KEY_BUDGETS_PERMISSION
                )
        })
        .cloned()
        .collect()
}

fn unsupported_warning(names: Vec<String>, supported: &[&str]) -> SettingWarning {
    let accepted: BTreeSet<&str> = supported
        .iter()
        .copied()
        .chain([PROJECTS_PERMISSION, MEMBER_KEY_BUDGETS_PERMISSION])
        .collect();
    let warning = SettingWarning::Unsupported {
        names,
        supported: accepted.iter().map(|name| (*name).to_owned()).collect(),
    };
    tracing::warn!("{}", warning.message());
    warning
}

// Whether team admins may edit budgets on the keys of other team members.
#[must_use]
pub fn may_edit_member_key_budgets(value: Option<&JsonValue>) -> bool {
    resolve_editable_fields(value, &[MEMBER_KEY_BUDGETS_PERMISSION])
        .fields
        .contains(MEMBER_KEY_BUDGETS_PERMISSION)
}
