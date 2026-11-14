use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::HeadroomError;
use super::request_data;
use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::headroom_messages::{protected_indices, retrieve_call_ids};
use crate::primitives::json_value::JsonValue;

pub(crate) struct Messages {
    pub(crate) all: Vec<JsonValue>,
    pub(crate) compressible: Vec<JsonValue>,
    pub(crate) protected: BTreeSet<usize>,
}

pub(crate) fn collect(
    inputs: &GuardrailApiInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<Messages>, HeadroomError> {
    let Some(structured) = inputs.structured_messages.as_deref() else {
        return Ok(None);
    };
    if structured.is_empty() {
        return Ok(None);
    }
    let all: Vec<JsonValue> = convert(structured)?;
    let raw_messages = request_data::untranslated_messages(request_data)?;
    let extra_ids = retrieve_call_ids(raw_messages.as_ref());
    let protected = protected_indices(&all, &extra_ids);
    let compressible: Vec<JsonValue> = all
        .iter()
        .enumerate()
        .filter(|(index, _row)| !protected.contains(index))
        .map(|(_index, row)| row.clone())
        .collect();
    if compressible.is_empty() {
        return Ok(None);
    }
    Ok(Some(Messages {
        all,
        compressible,
        protected,
    }))
}

pub(crate) fn convert<S, D>(source: &S) -> Result<D, HeadroomError>
where
    S: Serialize,
    D: DeserializeOwned,
{
    let encoded = serde_json::to_value(source)?;
    let converted = serde_json::from_value(encoded)?;
    Ok(converted)
}
