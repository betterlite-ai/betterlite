use crate::client::convert::ConvertError;
use crate::client::convert::parts::{to_compat_part, to_core_part};
use crate::client::records::compat::{Message, Role};
use crate::client::records::core::{CoreMessage, CoreRole};

pub fn to_core_message(message: Message) -> Result<CoreMessage, ConvertError> {
    let Message {
        role,
        parts,
        common,
    } = message;
    Ok(CoreMessage {
        role: match role {
            Role::User => CoreRole::User,
            Role::Agent => CoreRole::Agent,
        },
        parts: parts
            .into_iter()
            .map(to_core_part)
            .collect::<Result<Vec<_>, _>>()?,
        common,
    })
}

pub fn to_compat_message(message: &CoreMessage) -> Result<Message, ConvertError> {
    Ok(Message {
        role: compat_role(message.role),
        parts: message.parts.iter().map(to_compat_part).collect(),
        common: message.common.clone(),
    })
}

const fn compat_role(role: CoreRole) -> Role {
    match role {
        CoreRole::User => Role::User,
        CoreRole::Unspecified | CoreRole::Agent => Role::Agent,
    }
}
