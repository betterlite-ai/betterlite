use super::{DISABLE_SPEND_LOGS_PATH, MAX_BUDGET_PATH, budget_number};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{parse_money, resolve_number};
use crate::sections::settings::JsonValue;
use rust_decimal::Decimal;
use serde::Deserialize;

// The three YAML states of general_settings.disable_spend_logs, kept apart so an absent key never reads as a null the file wrote.
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
pub(super) enum SpendLogSwitch {
    Present(JsonValue),
    #[default]
    Absent,
}

// proxy_server's disable_spend_logs starts False, general_settings replaces it only when the key is there, and an explicit null is neither False nor True.
pub(super) fn disable_spend_logs(configured: &SpendLogSwitch) -> Result<Option<bool>, ConfigError> {
    match configured {
        SpendLogSwitch::Absent => Ok(Some(false)),
        SpendLogSwitch::Present(JsonValue::Null) => Ok(None),
        SpendLogSwitch::Present(JsonValue::Bool(flag)) => Ok(Some(*flag)),
        SpendLogSwitch::Present(
            JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        ) => Err(ConfigError::InvalidScalar {
            path: DISABLE_SPEND_LOGS_PATH.to_owned(),
            value: false,
        }),
    }
}

// litellm takes max_budget as a number or an os.environ reference; an absent one leaves zero
pub(super) fn max_budget<L>(
    configured: Option<JsonValue>,
    lookup: &L,
) -> Result<Decimal, ConfigError>
where
    L: EnvironmentLookup,
{
    let amount = resolve_number(
        configured.map(budget_number).transpose()?,
        MAX_BUDGET_PATH,
        lookup,
        parse_money,
    )?;
    Ok(amount.unwrap_or(Decimal::ZERO))
}
