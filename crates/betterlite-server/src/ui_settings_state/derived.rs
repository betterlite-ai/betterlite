use betterlite_config::loader::startup::process_environment;
use betterlite_config::ptu_pricing::PTU_COST_ATTRIBUTION_ENV_VAR;
use betterlite_config::sections::settings::settings_resolution::{FieldSource, source_for};
use betterlite_config::sections::settings::settings_rules::JsonValue;
use betterlite_config::sections::settings::settings_store::SettingsStore;
use betterlite_config::ui_settings::{
    PTU_COST_ATTRIBUTION_SETTING, USER_BUDGET_TEAM_KEYS_SETTING, UiSettingsDict, field_default,
};
use secrecy::ExposeSecret as _;

use crate::jobs::ptu::attribution::{EnvironmentAttribution, PtuAttribution};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DerivedUiSettings {
    pub enable_ptu_cost_attribution: bool,
    pub apply_user_budget_to_team_keys: bool,
}

// is_ptu_cost_attribution_enabled plus the secret read its source label needs.
pub trait PtuCostFlag: PtuAttribution {
    fn configured(&self) -> bool;
}

impl PtuCostFlag for EnvironmentAttribution {
    fn configured(&self) -> bool {
        let mut environment = process_environment();
        for (name, value) in &self.configured {
            let text = value
                .as_ref()
                .map_or_else(String::new, |secret| secret.expose_secret().to_owned());
            environment.insert(name.clone(), text);
        }
        environment.contains_key(PTU_COST_ATTRIBUTION_ENV_VAR)
    }
}

#[must_use]
pub fn derived_ui_settings<Flag: PtuCostFlag>(
    ptu: &Flag,
    general_settings: &UiSettingsDict,
) -> DerivedUiSettings {
    DerivedUiSettings {
        enable_ptu_cost_attribution: ptu.enabled(),
        apply_user_budget_to_team_keys: matches!(
            general_settings.get(USER_BUDGET_TEAM_KEYS_SETTING),
            Some(JsonValue::Bool(true))
        ),
    }
}

// _ui_setting_source: the two derived keys report config on their own rules, the rest through source_for.
#[must_use]
pub fn ui_setting_source<Flag: PtuCostFlag>(
    key: &str,
    value: &JsonValue,
    store: &SettingsStore,
    ptu: &Flag,
) -> FieldSource {
    if key == PTU_COST_ATTRIBUTION_SETTING {
        if ptu.configured() || value == &JsonValue::Bool(true) {
            return FieldSource::Config;
        }
        return FieldSource::Default;
    }
    if key == USER_BUDGET_TEAM_KEYS_SETTING {
        if value == &JsonValue::Bool(true) {
            return FieldSource::Config;
        }
        return FieldSource::Default;
    }
    let default = field_default(key);
    source_for(store, key, &default)
}
