use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;

use crate::agent_card::ProtocolVersion;
use crate::agent_card::merge::normalize_protocol_version;

use crate::version_convert::version_conversion::error::ConvertError;

use super::records::{Card, Interface};

pub(super) fn primary_legacy(card: &Card) -> Result<Interface, ConvertError> {
    let Some(interfaces) = &card.supported_interfaces else {
        return Err(ConvertError::NoLegacyInterface);
    };
    for interface in interfaces {
        if !legacy_version(interface.protocol_version.as_deref()) {
            continue;
        }
        return Ok(Interface {
            url: Some(interface.url.clone().unwrap_or_default()),
            protocol_binding: Some(interface.protocol_binding.clone().unwrap_or_default()),
            protocol_version: Some(
                interface
                    .protocol_version
                    .clone()
                    .filter(|text| !text.is_empty())
                    .unwrap_or_else(|| "0.3".to_owned()),
            ),
            tenant: None,
            passthrough: BTreeMap::new(),
        });
    }
    Err(ConvertError::NoLegacyInterface)
}

pub(super) fn upgrade(card: &Card) -> Vec<Interface> {
    let url = card.url.clone().unwrap_or_default();
    let transport = card
        .preferred_transport
        .clone()
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "JSONRPC".to_owned());
    let version = card
        .protocol_version
        .clone()
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "0.3".to_owned());
    let mut interfaces = vec![Interface {
        url: Some(url),
        protocol_binding: Some(transport),
        protocol_version: Some(version),
        tenant: None,
        passthrough: BTreeMap::new(),
    }];
    for additional in card.additional_interfaces.iter().flatten() {
        interfaces.push(Interface {
            url: additional.url.clone(),
            protocol_binding: additional.transport.clone(),
            protocol_version: Some("0.3".to_owned()),
            tenant: None,
            passthrough: BTreeMap::new(),
        });
    }
    interfaces
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
