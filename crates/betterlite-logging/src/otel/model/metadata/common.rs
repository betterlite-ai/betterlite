use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::model::utils::time::{FixedLocalTimeResolver, to_seconds};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::str::FromStr as _;

pub(super) const fn object(
    value: Option<&OtelModelValue>,
) -> Option<&BTreeMap<String, OtelModelValue>> {
    match value {
        Some(OtelModelValue::Object(fields)) => Some(fields),
        Some(
            OtelModelValue::Null(())
            | OtelModelValue::Bool(_)
            | OtelModelValue::Number(_)
            | OtelModelValue::String(_)
            | OtelModelValue::Array(_),
        )
        | None => None,
    }
}

pub(super) fn field<'value>(
    value: &'value OtelModelValue,
    key: &str,
) -> Option<&'value OtelModelValue> {
    match value {
        OtelModelValue::Object(fields) => fields.get(key),
        OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Number(_)
        | OtelModelValue::String(_)
        | OtelModelValue::Array(_) => None,
    }
}

pub(super) fn team_metadata(
    value: Option<&OtelModelValue>,
) -> Option<BTreeMap<String, OtelModelValue>> {
    object(value)
        .filter(|metadata| !metadata.is_empty())
        .cloned()
}

pub(super) fn string(value: Option<&OtelModelValue>) -> Option<String> {
    match value {
        Some(OtelModelValue::String(text)) if !text.is_empty() => Some(text.to_owned()),
        Some(
            OtelModelValue::Null(())
            | OtelModelValue::Bool(_)
            | OtelModelValue::Number(_)
            | OtelModelValue::String(_)
            | OtelModelValue::Array(_)
            | OtelModelValue::Object(_),
        )
        | None => None,
    }
}

pub(super) fn string_including_empty(value: Option<&OtelModelValue>) -> Option<String> {
    match value {
        Some(OtelModelValue::String(text)) => Some(text.to_owned()),
        Some(
            OtelModelValue::Null(())
            | OtelModelValue::Bool(_)
            | OtelModelValue::Number(_)
            | OtelModelValue::Array(_)
            | OtelModelValue::Object(_),
        )
        | None => None,
    }
}

pub(super) fn seconds(value: Option<&OtelModelValue>) -> Option<Decimal> {
    let offset = chrono::FixedOffset::east_opt(0)?;
    let resolver = FixedLocalTimeResolver { offset };
    let seconds = match value? {
        OtelModelValue::Number(number) => {
            let decimal = Decimal::from_str(&number.to_string()).ok()?;
            to_seconds(Some(&decimal), None, None, &resolver)?
        }
        OtelModelValue::String(text) => to_seconds(None, None, Some(text), &resolver)?,
        OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Array(_)
        | OtelModelValue::Object(_) => return None,
    };
    Some(seconds)
}

pub(super) fn scalar_text(value: &OtelModelValue) -> Option<String> {
    match value {
        OtelModelValue::String(text) => Some(text.clone()),
        OtelModelValue::Bool(true) => Some("True".to_owned()),
        OtelModelValue::Bool(false) => Some("False".to_owned()),
        OtelModelValue::Number(number) => Some(number.to_string()),
        OtelModelValue::Null(()) | OtelModelValue::Array(_) | OtelModelValue::Object(_) => None,
    }
}
