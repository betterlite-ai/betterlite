use betterlite_core::agents::json::AgentJson;

use crate::version_convert::JsonObject;

use super::error::ConvertError;
use super::json::encode;

mod interfaces;
mod records;

use records::{Capabilities, Card};

pub(super) fn upgrade(card: &JsonObject) -> Result<JsonObject, ConvertError> {
    let mut record = parse(card)?;
    require_03_members(&record)?;
    record.supported_interfaces = Some(interfaces::upgrade(&record));
    record.protocol_version = None;
    record.url = None;
    record.preferred_transport = None;
    record.additional_interfaces = None;
    record.security_requirements = record.security.take();
    let extended = take_extended(&mut record);
    let Some(mut capabilities) = record.capabilities.take() else {
        return Err(ConvertError::MissingCardField {
            field: "capabilities",
        });
    };
    capabilities.extended_agent_card = extended;
    capabilities.extensions = capabilities.extensions.filter(|items| !items.is_empty());
    record.capabilities = Some(capabilities);
    record.extended_card_support = None;
    drop_empty(&mut record);
    encode(&record)
}

fn require_03_members(record: &Card) -> Result<(), ConvertError> {
    let required = [
        ("name", record.name.is_some()),
        ("description", record.description.is_some()),
        ("version", record.version.is_some()),
        ("url", record.url.is_some()),
        ("capabilities", record.capabilities.is_some()),
        ("defaultInputModes", record.default_input_modes.is_some()),
        ("defaultOutputModes", record.default_output_modes.is_some()),
        ("skills", record.skills.is_some()),
    ];
    for (field, present) in required {
        if !present {
            return Err(ConvertError::MissingCardField { field });
        }
    }
    Ok(())
}

fn drop_empty(record: &mut Card) {
    record.security_requirements = record
        .security_requirements
        .take()
        .filter(|items| !items.is_empty());
    record.security_schemes = record
        .security_schemes
        .take()
        .filter(|schemes| !schemes.is_empty());
    record.default_input_modes = record
        .default_input_modes
        .take()
        .filter(|items| !items.is_empty());
    record.default_output_modes = record
        .default_output_modes
        .take()
        .filter(|items| !items.is_empty());
    record.skills = record.skills.take().filter(|items| !items.is_empty());
    record.signatures = record.signatures.take().filter(|items| !items.is_empty());
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
    record.name.get_or_insert_default();
    record.description.get_or_insert_default();
    record.version.get_or_insert_default();
    record.default_input_modes.get_or_insert_default();
    record.default_output_modes.get_or_insert_default();
    record.skills.get_or_insert_default();
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
