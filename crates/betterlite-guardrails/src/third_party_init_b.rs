pub mod error;
pub mod panw;
pub mod pillar;
pub mod rubrik;
pub mod straiker;
pub mod typesafe;
pub mod vigil_guard;
pub mod xecguard;

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use serde::de::DeserializeOwned;

use self::error::ThirdPartyInitBError;

pub(crate) type InitResult<T> = Result<T, ThirdPartyInitBError>;

pub(crate) fn decode<T>(name: &str, value: &GuardrailValue) -> InitResult<T>
where
    T: DeserializeOwned,
{
    let encoded =
        serde_json::to_vec(value).map_err(|_not_json| ThirdPartyInitBError::InvalidParam {
            name: name.to_owned(),
        })?;
    serde_json::from_slice(&encoded).map_err(|_wrong_type| ThirdPartyInitBError::InvalidParam {
        name: name.to_owned(),
    })
}

pub(crate) fn read_extra<T>(extra: &OpaqueParams, name: &str) -> InitResult<Option<T>>
where
    T: DeserializeOwned,
{
    match extra.0.get(name) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(value) => decode(name, value).map(Some),
    }
}

// LiteLLM's fields with a model default: an absent key takes it, an explicit null stays null
pub(crate) fn read_extra_default<T>(
    extra: &OpaqueParams,
    name: &str,
    default: T,
) -> InitResult<Option<T>>
where
    T: DeserializeOwned,
{
    match extra.0.get(name) {
        None => Ok(Some(default)),
        Some(GuardrailValue::Null) => Ok(None),
        Some(value) => decode(name, value).map(Some),
    }
}

// LiteLLM's optional_params: the mapping the config wrote, none when it wrote an absent or null one
pub(crate) struct OptionalParams<'a> {
    entries: Option<&'a BTreeMap<String, GuardrailValue>>,
}

impl<'a> OptionalParams<'a> {
    pub(crate) fn parse(extra: &'a OpaqueParams) -> InitResult<Self> {
        match extra.0.get("optional_params") {
            None | Some(GuardrailValue::Null) => Ok(Self { entries: None }),
            Some(GuardrailValue::Object(entries)) => Ok(Self {
                entries: Some(entries),
            }),
            Some(_wrong) => Err(ThirdPartyInitBError::InvalidParam {
                name: "optional_params".to_owned(),
            }),
        }
    }

    pub(crate) fn read<T>(&self, name: &str) -> InitResult<Option<T>>
    where
        T: DeserializeOwned,
    {
        match self.entries.and_then(|entries| entries.get(name)) {
            None | Some(GuardrailValue::Null) => Ok(None),
            Some(value) => decode(name, value).map(Some),
        }
    }
}

// LiteLLM's _get_config_value: the optional_params entry when it is non-null, else the top-level param
pub(crate) fn read_config_value<T>(
    extra: &OpaqueParams,
    optional: &OptionalParams<'_>,
    name: &str,
) -> InitResult<Option<T>>
where
    T: DeserializeOwned,
{
    match optional.read(name)? {
        Some(value) => Ok(Some(value)),
        None => read_extra(extra, name),
    }
}

pub(crate) fn read_config_default<T>(
    extra: &OpaqueParams,
    optional: &OptionalParams<'_>,
    name: &str,
    default: T,
) -> InitResult<Option<T>>
where
    T: DeserializeOwned,
{
    match optional.read(name)? {
        Some(value) => Ok(Some(value)),
        None => read_extra_default(extra, name, default),
    }
}

// the class reads the literal spelling, not the wire enum, so the text keeps LiteLLM's values
pub(crate) const fn unreachable_fallback_text(fallback: UnreachableFallback) -> &'static str {
    match fallback {
        UnreachableFallback::FailClosed => "fail_closed",
        UnreachableFallback::FailOpen => "fail_open",
    }
}
