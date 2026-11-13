use crate::media::error::MediaError;
use crate::message_content::{Content, Message};

// The same message and part ceilings B234 puts on the traversal's Message list
pub const MAX_MESSAGES: usize = 1024;
pub const MAX_MESSAGE_PARTS: usize = 256;
// One message's worth of parts bounds the distinct media a request can make the gateway download
pub const MAX_REMOTE_URLS: usize = 256;

pub(crate) fn check_messages(messages: &[Message]) -> Result<(), MediaError> {
    if messages.len() > MAX_MESSAGES {
        return Err(MediaError::TooManyMessages {
            limit: MAX_MESSAGES,
        });
    }
    for message in messages {
        let Some(Content::Parts(parts)) = &message.content else {
            continue;
        };
        if parts.len() > MAX_MESSAGE_PARTS {
            return Err(MediaError::TooManyMessageParts {
                limit: MAX_MESSAGE_PARTS,
            });
        }
    }
    Ok(())
}
