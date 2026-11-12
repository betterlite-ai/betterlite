use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::Number;

use crate::decisions::Json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropFlag {
    pub enabled: bool,
    pub warned: bool,
}

#[must_use]
pub fn normalize_drop_params(value: Option<&Json>) -> Option<bool> {
    match value? {
        Json::Bool(flag) => Some(*flag),
        // the shared parser owns text-to-bool, and pydantic's short spellings outside it count as invalid
        Json::Text(text) => betterlite_env::flag("drop_params", text).unwrap_or(None),
        Json::Number(number) => flag_from_number(number),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

#[must_use]
pub fn drop_params_flag(value: Option<&Json>) -> DropFlag {
    let normalized = normalize_drop_params(value);
    DropFlag {
        enabled: normalized.unwrap_or(false),
        warned: normalized.is_none() && value.is_some_and(|given| !matches!(given, Json::Null)),
    }
}

#[must_use]
pub fn drop_params_env_flag(configured: Option<&str>) -> DropFlag {
    let text = configured.unwrap_or_default().trim();
    if text.is_empty() {
        return DropFlag {
            enabled: false,
            warned: false,
        };
    }
    betterlite_env::flag("LITELLM_DROP_PARAMS", text)
        .unwrap_or(None)
        .map_or(
            DropFlag {
                enabled: true,
                warned: true,
            },
            |enabled| DropFlag {
                enabled,
                warned: false,
            },
        )
}

fn flag_from_number(number: &Number) -> Option<bool> {
    if let Some(integer) = number.as_i64() {
        return match integer {
            0_i64 => Some(false),
            1_i64 => Some(true),
            _ => None,
        };
    }
    let decimal = Decimal::from_f64(number.as_f64()?)?;
    if decimal.is_zero() {
        Some(false)
    } else if decimal == Decimal::ONE {
        Some(true)
    } else {
        None
    }
}
