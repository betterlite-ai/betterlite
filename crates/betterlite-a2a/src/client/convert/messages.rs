use crate::client::convert::ConvertError;
use crate::client::convert::parts::{to_compat_part, to_core_part};
use crate::client::records::compat::{Message, MessageKind, Role};
use crate::client::records::core::{CoreMessage, CoreRole};

pub fn to_core_message(message: Message) -> Result<CoreMessage, ConvertError> {
    let Message {
        role,
        parts,
        common,
        ..
    } = message;
    let mut converted = Vec::with_capacity(parts.len());
    for part in parts {
        converted.push(to_core_part(part)?);
    }
    Ok(CoreMessage {
        role: core_role(role),
        parts: converted,
        common,
    })
}

pub fn to_compat_message(message: &CoreMessage) -> Message {
    let parts = message.parts.iter().map(to_compat_part).collect();
    Message {
        kind: MessageKind::Message,
        role: compat_role(message.role),
        parts,
        common: message.common.clone(),
    }
}

const fn core_role(role: Role) -> CoreRole {
    match role {
        Role::User => CoreRole::User,
        Role::Agent => CoreRole::Agent,
    }
}

const fn compat_role(role: CoreRole) -> Role {
    match role {
        CoreRole::User => Role::User,
        CoreRole::Unspecified | CoreRole::Agent => Role::Agent,
    }
}
