use crate::straiker::v3::PLACEHOLDER_IDENTITIES;
use crate::straiker_webhook::types::StraikerWebhookIdentity;

// LiteLLM's proxy-admin placeholders are not a person.
#[must_use]
pub fn real_identity(value: Option<&str>) -> Option<&str> {
    let identity = value?;
    if identity.is_empty() || PLACEHOLDER_IDENTITIES.contains(&identity) {
        return None;
    }
    Some(identity)
}

// The key is the authenticated principal, so its own user comes before the end user the request named.
#[must_use]
pub fn user(identity: &StraikerWebhookIdentity) -> Option<String> {
    [
        &identity.litellm_user_email,
        &identity.litellm_user_id,
        &identity.end_user_id,
    ]
    .into_iter()
    .find_map(|candidate| real_identity(candidate.as_deref()))
    .map(str::to_owned)
}

mod tests;
