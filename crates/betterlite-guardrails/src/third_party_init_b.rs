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

pub(crate) fn decode<T>(name: &str, value: Option<&GuardrailValue>) -> InitResult<Option<T>>
where
    T: DeserializeOwned,
{
    let Some(present) = value.filter(|entry| **entry != GuardrailValue::Null) else {
        return Ok(None);
    };
    let encoded =
        serde_json::to_vec(present).map_err(|_not_json| ThirdPartyInitBError::InvalidParam {
            name: name.to_owned(),
        })?;
    serde_json::from_slice(&encoded)
        .map(Some)
        .map_err(|_wrong_type| ThirdPartyInitBError::InvalidParam {
            name: name.to_owned(),
        })
}

// what _get_config_value sees: every optional_params entry that is not null laid over the top-level params
pub(crate) struct ParamsView<'params> {
    pub(crate) merged: BTreeMap<String, GuardrailValue>,
    pub(crate) optional: Option<&'params BTreeMap<String, GuardrailValue>>,
}

impl<'params> ParamsView<'params> {
    pub(crate) fn parse(extra: &'params OpaqueParams) -> InitResult<Self> {
        let optional = match extra.0.get("optional_params") {
            None | Some(GuardrailValue::Null) => None,
            Some(GuardrailValue::Object(entries)) => Some(entries),
            Some(_wrong) => {
                return Err(ThirdPartyInitBError::InvalidParam {
                    name: "optional_params".to_owned(),
                });
            }
        };
        let mut merged = extra.0.clone();
        for (name, value) in optional
            .into_iter()
            .flatten()
            .filter(|(_, value)| **value != GuardrailValue::Null)
        {
            merged.insert(name.clone(), value.clone());
        }
        Ok(Self { merged, optional })
    }
}

// a top-level param that LiteLLM's model gives a default: absent takes it, an explicit null stays null
pub(crate) fn defaulted<T>(view: &ParamsView<'_>, name: &str, default: T) -> InitResult<Option<T>>
where
    T: DeserializeOwned,
{
    view.merged
        .get(name)
        .map_or(Ok(Some(default)), |value| decode(name, Some(value)))
}

// the class reads the literal spelling, not the wire enum, so the text keeps LiteLLM's values
pub(crate) const fn unreachable_fallback_text(fallback: UnreachableFallback) -> &'static str {
    match fallback {
        UnreachableFallback::FailClosed => "fail_closed",
        UnreachableFallback::FailOpen => "fail_open",
    }
}
