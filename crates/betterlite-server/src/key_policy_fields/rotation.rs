use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use crate::key_policy_fields::{ClockError, KeyClock, shifted_by, zulu_datetime};

pub const ALIAS_REQUIRED_MESSAGE: &str = "key_alias is required when auto_rotate=True and store_virtual_keys is enabled. This ensures stable secret naming during rotation.";

#[derive(Debug, thiserror::Error)]
pub enum RotationError {
    #[error(transparent)]
    Clock(#[from] ClockError),
    #[error("{0}")]
    AliasRequired(Box<ProxyException>),
}

// rotation fields change only when rotation is switched on with an interval; turning it off keeps them
pub fn set_key_rotation_fields(
    data: &mut BTreeMap<String, JsonData>,
    auto_rotate: bool,
    rotation_interval: Option<&str>,
    existing_key_alias: Option<&str>,
    store_virtual_keys: bool,
    clock: KeyClock<'_>,
) -> Result<(), RotationError> {
    if !auto_rotate {
        return Ok(());
    }
    let Some(interval) = rotation_interval.filter(|text| !text.is_empty()) else {
        return Ok(());
    };
    let requested_alias = data.get("key_alias");
    let requested_alias_is_missing = matches!(requested_alias, None | Some(JsonData::Null));
    if store_virtual_keys && requested_alias_is_missing && existing_key_alias.is_none() {
        return Err(RotationError::AliasRequired(alias_required()));
    }
    let rotation_at = shifted_by(clock.now, interval, clock)?;
    data.insert("auto_rotate".to_owned(), JsonData::Bool(true));
    data.insert(
        "rotation_interval".to_owned(),
        JsonData::String(interval.to_owned()),
    );
    data.insert(
        "key_rotation_at".to_owned(),
        JsonData::String(zulu_datetime(rotation_at)),
    );
    Ok(())
}

fn alias_required() -> Box<ProxyException> {
    Box::new(ProxyException::new(ProxyExceptionInit {
        message: ALIAS_REQUIRED_MESSAGE.to_owned(),
        error_type: ProxyErrorTypes::BadRequestError.to_string(),
        param: Some("key_alias".to_owned()),
        code: ExceptionCode::Status(400),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    }))
}
