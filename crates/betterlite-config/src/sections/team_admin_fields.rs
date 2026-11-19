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
#[derive(Debug, Clone, Default, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolution {
    pub fields: BTreeSet<String>,
    pub warnings: Vec<SettingWarning>,
}

pub fn resolve_editable_fields(value: Option<&JsonValue>, supported: &[&str]) -> Resolution {
    let Some(raw) = value.filter(|value| !value.is_null()) else {
        return Resolution::default();
    };
    let configured = match raw {
        JsonValue::Array(items) => items
            .iter()
            .map(|item| match item {
                JsonValue::String(name) => Some(name.clone()),
                _ => None,
            })
            .collect::<Option<BTreeSet<String>>>(),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Object(_) => None,
    };
    let Some(configured) = configured else {
        let warning = SettingWarning::NotAList { raw: raw.clone() };
        tracing::warn!("{}", warning.message());
        return Resolution {
            fields: BTreeSet::new(),
            warnings: vec![warning],
        };
    };
    let unsupported: Vec<String> = configured
        .iter()
        .filter(|name| {
            !supported.contains(&name.as_str())
                && !matches!(
                    name.as_str(),
                    PROJECTS_PERMISSION | MEMBER_KEY_BUDGETS_PERMISSION
                )
        })
        .cloned()
        .collect();
    let mut warnings = Vec::new();
    if !unsupported.is_empty() {
        let accepted: BTreeSet<&str> = supported
            .iter()
            .copied()
            .chain([PROJECTS_PERMISSION, MEMBER_KEY_BUDGETS_PERMISSION])
            .collect();
        let warning = SettingWarning::Unsupported {
            names: unsupported,
            supported: accepted.iter().map(|name| (*name).to_owned()).collect(),
        };
        tracing::warn!("{}", warning.message());
        warnings.push(warning);
    }
    let mut fields = BTreeSet::new();
    for name in &configured {
        if supported.contains(&name.as_str()) {
            fields.insert(name.clone());
        }
    }
    Resolution { fields, warnings }
}

// Whether team admins may edit budgets on the keys of other team members.
pub fn may_edit_member_key_budgets(value: Option<&JsonValue>) -> bool {
    resolve_editable_fields(value, &[MEMBER_KEY_BUDGETS_PERMISSION])
        .fields
        .contains(MEMBER_KEY_BUDGETS_PERMISSION)
}
