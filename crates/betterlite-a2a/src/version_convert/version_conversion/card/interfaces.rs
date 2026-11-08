use betterlite_core::agents::json::AgentJson;

use crate::agent_card::ProtocolVersion;
use crate::agent_card::merge::normalize_protocol_version;
use crate::version_convert::{JsonObject, to_object};

use crate::version_convert::version_conversion::error::ConvertError;
use crate::version_convert::version_conversion::fields::{array, string};

pub(super) struct LegacyInterface {
    pub(super) url: String,
    pub(super) transport: String,
    pub(super) protocol_version: String,
}

pub(super) fn primary_legacy(card: &JsonObject) -> Result<LegacyInterface, ConvertError> {
    let interfaces = array(card, "supportedInterfaces")?.unwrap_or_default();
    for interface in interfaces {
        let Some(fields) = to_object(&interface) else {
            return Err(ConvertError::Field {
                field: "supportedInterfaces".to_owned(),
                expected: "an array of interface objects",
            });
        };
        let protocol_version = string(&fields, "protocolVersion")?;
        if !legacy_version(protocol_version.as_deref()) {
            continue;
        }
        return Ok(LegacyInterface {
            url: string(&fields, "url")?.unwrap_or_default(),
            transport: string(&fields, "protocolBinding")?.unwrap_or_default(),
            protocol_version: protocol_version
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| "0.3".to_owned()),
        });
    }
    Err(ConvertError::NoLegacyInterface)
}

pub(super) fn upgrade(card: &JsonObject) -> Result<AgentJson, ConvertError> {
    let url = string(card, "url")?.unwrap_or_default();
    let transport = string(card, "preferredTransport")?
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "JSONRPC".to_owned());
    let version = string(card, "protocolVersion")?
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "0.3".to_owned());
    let mut interfaces = vec![interface(&url, &transport, &version)];
    for additional in array(card, "additionalInterfaces")?.unwrap_or_default() {
        let Some(fields) = to_object(&additional) else {
            return Err(ConvertError::Field {
                field: "additionalInterfaces".to_owned(),
                expected: "an array of interface objects",
            });
        };
        let additional_url = string(&fields, "url")?.unwrap_or_default();
        let additional_transport = string(&fields, "transport")?.unwrap_or_default();
        interfaces.push(interface(&additional_url, &additional_transport, "0.3"));
    }
    Ok(AgentJson::Array(interfaces))
}

fn legacy_version(version: Option<&str>) -> bool {
    match version {
        None | Some("") => true,
        Some(text) => {
            let value = AgentJson::String(text.to_owned());
            normalize_protocol_version(&value) == Some(ProtocolVersion::V0_3)
        }
    }
}

fn interface(url: &str, binding: &str, protocol_version: &str) -> AgentJson {
    let mut fields = JsonObject::new();
    fields.insert("url".to_owned(), AgentJson::String(url.to_owned()));
    fields.insert(
        "protocolBinding".to_owned(),
        AgentJson::String(binding.to_owned()),
    );
    fields.insert(
        "protocolVersion".to_owned(),
        AgentJson::String(protocol_version.to_owned()),
    );
    AgentJson::Object(fields.into_iter().collect())
}
