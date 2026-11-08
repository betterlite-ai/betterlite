use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

pub const API_KEY_VARIABLE: &str = "LAGO_API_KEY";
pub const API_BASE_VARIABLE: &str = "LAGO_API_BASE";
pub const EVENT_CODE_VARIABLE: &str = "LAGO_API_EVENT_CODE";
pub const CHARGE_BY_VARIABLE: &str = "LAGO_API_CHARGE_BY";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChargeBy {
    #[default]
    EndUserId,
    TeamId,
    UserId,
}

impl ChargeBy {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "end_user_id" => Some(Self::EndUserId),
            "team_id" => Some(Self::TeamId),
            "user_id" => Some(Self::UserId),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct LagoSettings {
    pub api_base: String,
    pub api_key: SecretString,
    pub event_code: String,
    pub charge_by: ChargeBy,
}

#[derive(Debug, thiserror::Error)]
pub enum LagoSettingsError {
    #[error("Lago logging needs these environment variables: {missing_keys:?}")]
    MissingKeys { missing_keys: Vec<&'static str> },
    #[error("LAGO_API_CHARGE_BY is {value:?}; it must be end_user_id, team_id or user_id")]
    InvalidChargeBy { value: String },
}

impl LagoSettings {
    pub fn from_environment(
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, LagoSettingsError> {
        let api_key = environment
            .lookup(API_KEY_VARIABLE)
            .filter(|value| !value.is_empty());
        let api_base = environment
            .lookup(API_BASE_VARIABLE)
            .filter(|value| !value.is_empty());
        let event_code = environment
            .lookup(EVENT_CODE_VARIABLE)
            .filter(|value| !value.is_empty());
        let missing_keys: Vec<&'static str> = [
            (API_KEY_VARIABLE, api_key.is_none()),
            (API_BASE_VARIABLE, api_base.is_none()),
            (EVENT_CODE_VARIABLE, event_code.is_none()),
        ]
        .into_iter()
        .filter_map(|(name, missing)| missing.then_some(name))
        .collect();
        match (api_key, api_base, event_code) {
            (Some(api_key_value), Some(api_base_value), Some(event_code_value)) => Ok(Self {
                api_base: api_base_value,
                api_key: SecretString::from(api_key_value),
                event_code: event_code_value,
                charge_by: charge_by(environment)?,
            }),
            _ => Err(LagoSettingsError::MissingKeys { missing_keys }),
        }
    }
}

fn charge_by(environment: &dyn EnvironmentLookup) -> Result<ChargeBy, LagoSettingsError> {
    match environment.lookup(CHARGE_BY_VARIABLE) {
        None => Ok(ChargeBy::EndUserId),
        Some(value) if value.is_empty() => Ok(ChargeBy::EndUserId),
        Some(value) => ChargeBy::parse(&value).ok_or(LagoSettingsError::InvalidChargeBy { value }),
    }
}
