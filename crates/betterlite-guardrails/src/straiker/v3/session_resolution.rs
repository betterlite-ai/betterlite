use betterlite_prompts::arguments::json::Object;
use sha2::{Digest as _, Sha256};

use crate::straiker::v3::error::V3Error;
use crate::straiker::v3::first_user_text::first_message_text;
use crate::straiker::v3::request_header::inbound_header;
use crate::straiker::v3::system_text::system_text;
use crate::straiker::v3::user_identity::user;
use crate::straiker::v3::{DERIVED_SESSION_PREFIX, SESSION_HEADER};
use crate::straiker_webhook::types::{StraikerWebhookContext, StraikerWebhookIdentity};

const DERIVED_SESSION_HEX_CHARS: usize = 32;

// Kong's order: the session the client names on the wire, then the one LiteLLM resolved from its own metadata, then a hash of the principal, the system prompt and the first message. A chat client replays the whole conversation on every turn, so that triple is constant for its lifetime. The principal is in the hash because Straiker skips turns it already scored for a session: two users who open with the same words must be two conversations.
pub fn session_id(
    context: &StraikerWebhookContext,
    identity: &StraikerWebhookIdentity,
    request_data: &Object,
    request_body: &Object,
) -> Result<Option<String>, V3Error> {
    if let Some(supplied) = inbound_header(request_data, SESSION_HEADER) {
        return Ok(Some(supplied));
    }
    if let Some(resolved) = context
        .session_id
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        return Ok(Some(resolved.to_owned()));
    }
    let system = system_text(request_body)?.unwrap_or_default();
    let first_message = first_message_text(request_body);
    if system.is_empty() && first_message.is_empty() {
        return Ok(None);
    }
    let principal = user(identity).unwrap_or_default();
    let seed = format!("{principal}\0{system}\0{first_message}");
    let digest = hex::encode(Sha256::digest(seed.as_bytes()));
    let short: String = digest.chars().take(DERIVED_SESSION_HEX_CHARS).collect();
    Ok(Some(format!("{DERIVED_SESSION_PREFIX}{short}")))
}

mod tests;
