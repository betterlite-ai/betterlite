use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_exception::ProxyException;

use crate::key_policy_fields::{
    ClockError, KeyClock, bad_request_for_alias, shifted_by, zulu_datetime,
};

pub const ALIAS_REQUIRED_MESSAGE: &str = "key_alias is required when auto_rotate=True and store_virtual_keys is enabled. This ensures stable secret naming during rotation.";

#[derive(Debug, thiserror::Error)]
pub enum RotationError {
    #[error(transparent)]
    Clock(#[from] ClockError),
    #[error("{0}")]
    AliasRequired(Box<ProxyException>),
}

// the two switches rotation depends on: the request's auto_rotate and the proxy's store_virtual_keys
#[derive(Debug, Clone, Copy)]
pub struct RotationSwitches {
    pub auto_rotate: bool,
    pub store_virtual_keys: bool,
}

// rotation fields change only when rotation is switched on with an interval; turning it off keeps them
pub fn set_key_rotation_fields(
    data: &mut BTreeMap<String, JsonData>,
    switches: RotationSwitches,
    rotation_interval: Option<&str>,
    existing_key_alias: Option<&str>,
    clock: KeyClock<'_>,
) -> Result<(), RotationError> {
    if !switches.auto_rotate {
        return Ok(());
    }
    let Some(interval) = rotation_interval.filter(|text| !text.is_empty()) else {
        return Ok(());
    };
    let requested_alias_is_missing = matches!(data.get("key_alias"), None | Some(JsonData::Null));
    if switches.store_virtual_keys && requested_alias_is_missing && existing_key_alias.is_none() {
        return Err(RotationError::AliasRequired(bad_request_for_alias(
            ALIAS_REQUIRED_MESSAGE.to_owned(),
        )));
    }
    let rotation_at = shifted_by(clock.now, interval, clock)?;
    data.extend(BTreeMap::from([
        ("auto_rotate".to_owned(), JsonData::Bool(true)),
        (
            "rotation_interval".to_owned(),
            JsonData::String(interval.to_owned()),
        ),
        (
            "key_rotation_at".to_owned(),
            JsonData::String(zulu_datetime(rotation_at)),
        ),
    ]));
    Ok(())
}
