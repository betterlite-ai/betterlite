use std::iter::empty;

use betterlite_core::agents::json::AgentJson;

use crate::version_convert::JsonObject;

use super::error::ConvertError;

mod interfaces;

enum Direction {
    Upgrade,
    Downgrade,
}

pub(super) fn upgrade(card: &JsonObject) -> Result<JsonObject, ConvertError> {
    let mut out = card.clone();
    drop_keys(
        &mut out,
        &[
            "url",
            "preferredTransport",
            "protocolVersion",
            "additionalInterfaces",
        ],
    );
    out.insert("supportedInterfaces".to_owned(), interfaces::upgrade(card)?);
    move_field(&mut out, "security", "securityRequirements");
    move_extended_flag(&mut out, &Direction::Upgrade);
    // a 0.3 card requires capabilities, so a card without one cannot be converted
    if !matches!(out.get("capabilities"), Some(AgentJson::Object(_))) {
        return Err(ConvertError::Field {
            field: "capabilities".to_owned(),
            expected: "an object",
        });
    }
    Ok(out)
}

pub(super) fn downgrade(card: &JsonObject) -> Result<JsonObject, ConvertError> {
    let primary = interfaces::primary_legacy(card)?;
    let mut out = card.clone();
    drop_keys(&mut out, &["supportedInterfaces", "additionalInterfaces"]);
    out.insert("url".to_owned(), AgentJson::String(primary.url));
    out.insert(
        "preferredTransport".to_owned(),
        AgentJson::String(primary.transport),
    );
    out.insert(
        "protocolVersion".to_owned(),
        AgentJson::String(primary.protocol_version),
    );
    move_field(&mut out, "securityRequirements", "security");
    move_extended_flag(&mut out, &Direction::Downgrade);
    if !matches!(out.get("capabilities"), Some(AgentJson::Object(_))) {
        out.insert(
            "capabilities".to_owned(),
            AgentJson::Object(empty().collect()),
        );
    }
    Ok(out)
}

fn drop_keys(fields: &mut JsonObject, keys: &[&str]) {
    for key in keys {
        fields.remove(*key);
    }
}

fn move_field(fields: &mut JsonObject, from: &str, to: &str) {
    if let Some(value) = fields.remove(from) {
        fields.insert(to.to_owned(), value);
    }
}

fn move_extended_flag(fields: &mut JsonObject, direction: &Direction) {
    match direction {
        Direction::Upgrade => {
            if let Some(flag) = fields.remove("supportsAuthenticatedExtendedCard") {
                insert_capability(fields, "extendedAgentCard", flag);
            }
        }
        Direction::Downgrade => {
            if let Some(flag) = remove_capability(fields, "extendedAgentCard") {
                fields.insert("supportsAuthenticatedExtendedCard".to_owned(), flag);
            }
        }
    }
}

fn insert_capability(card: &mut JsonObject, key: &str, value: AgentJson) {
    let mut capabilities = match card.remove("capabilities") {
        Some(AgentJson::Object(fields)) => fields,
        Some(_) | None => empty().collect(),
    };
    capabilities.insert(key.to_owned(), value);
    card.insert("capabilities".to_owned(), AgentJson::Object(capabilities));
}

fn remove_capability(card: &mut JsonObject, key: &str) -> Option<AgentJson> {
    let Some(AgentJson::Object(mut capabilities)) = card.remove("capabilities") else {
        return None;
    };
    let value = capabilities.remove(key);
    card.insert("capabilities".to_owned(), AgentJson::Object(capabilities));
    value
}
