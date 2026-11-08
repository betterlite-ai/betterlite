use betterlite_core::agents::json::AgentJson;

use crate::version_convert::JsonObject;

use super::error::ConvertError;
use super::json::encode;

mod interfaces;
mod records;

use records::{Capabilities, Card};

pub(super) fn upgrade(card: &JsonObject) -> Result<JsonObject, ConvertError> {
    let mut record = parse(card)?;
    record.supported_interfaces = Some(interfaces::upgrade(&record));
    record.protocol_version = None;
    record.url = None;
    record.preferred_transport = None;
    record.additional_interfaces = None;
    record.security_requirements = record.security.take();
    let extended = take_extended(&mut record);
    let Some(mut capabilities) = record.capabilities.take() else {
        return Err(ConvertError::Field {
            field: "capabilities".to_owned(),
            expected: "an object",
        });
    };
    capabilities.extended_agent_card = extended;
    record.capabilities = Some(capabilities);
    record.extended_card_support = None;
    encode(&record)
}

pub(super) fn downgrade(card: &JsonObject) -> Result<JsonObject, ConvertError> {
    let mut record = parse(card)?;
    let primary = interfaces::primary_legacy(&record)?;
    record.supported_interfaces = None;
    record.additional_interfaces = None;
    // the 0.3 card schema defines no other top-level keys, so unknown ones are dropped
    record.passthrough.clear();
    record.url = primary.url;
    record.preferred_transport = primary.protocol_binding;
    record.protocol_version = primary.protocol_version;
    record.security = record.security_requirements.take();
    record.extended_card_support = take_extended(&mut record);
    if record.capabilities.is_none() {
        record.capabilities = Some(Capabilities::default());
    }
    encode(&record)
}

fn parse(card: &JsonObject) -> Result<Card, ConvertError> {
    let encoded = AgentJson::Object(card.clone().into_iter().collect());
    let decoded: Card = serde_json::from_value(encoded).map_err(ConvertError::Decode)?;
    Ok(decoded)
}

fn take_extended(record: &mut Card) -> Option<bool> {
    let capabilities = record.capabilities.as_mut()?;
    capabilities.state_transition_history = None;
    capabilities.extended_agent_card.take()
}
