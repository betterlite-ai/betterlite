use super::{DISABLE_SPEND_LOGS_PATH, MAX_BUDGET_PATH, budget_number};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{parse_money, resolve_number};
use crate::sections::settings::JsonValue;
use rust_decimal::Decimal;

// proxy_server's disable_spend_logs starts False, general_settings replaces it only when the key is there, and an explicit null is neither False nor True.
pub(super) fn disable_spend_logs(
    configured: &Option<Option<JsonValue>>,
) -> Result<Option<bool>, ConfigError> {
    let Some(present) = configured else {
        return Ok(Some(false));
    };
    let Some(value) = present else {
        return Ok(None);
    };
    if let JsonValue::Bool(flag) = value {
        return Ok(Some(*flag));
    }
    Err(ConfigError::InvalidScalar {
        path: DISABLE_SPEND_LOGS_PATH.to_owned(),
        value: false,
    })
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
