use betterlite_prompts::arguments::json::{Json, Object};

use crate::purview_graph_client::PurviewGuardrailBase;

#[derive(Debug, Clone, Copy, Default)]
pub struct KeyIdentity<'req> {
    pub user_id: Option<&'req str>,
    pub end_user_id: Option<&'req str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedUserId {
    Trusted(String),
    Untrusted(String),
}

impl ResolvedUserId {
    // Only the key or JWT bound id may reach a blocking DLP check; every other source is a caller hint.
    #[must_use]
    pub fn into_trusted(self) -> Option<String> {
        match self {
            Self::Trusted(user_id) => Some(user_id),
            Self::Untrusted(_) => None,
        }
    }
}

fn non_empty_text<'obj>(object: &'obj Object, key: &str) -> Option<&'obj str> {
    object
        .get(key)
        .and_then(Json::as_str)
        .filter(|text| !text.is_empty())
}

// Walks key user id, end user id, proxy-injected metadata, then the configured metadata field.
#[must_use]
pub fn resolve_user_id(
    base: &PurviewGuardrailBase,
    data: &Object,
    identity: &KeyIdentity<'_>,
) -> Option<ResolvedUserId> {
    if let Some(user_id) = identity.user_id.filter(|user_id| !user_id.is_empty()) {
        return Some(ResolvedUserId::Trusted(user_id.to_owned()));
    }
    if let Some(end_user_id) = identity.end_user_id.filter(|user_id| !user_id.is_empty()) {
        return Some(ResolvedUserId::Untrusted(end_user_id.to_owned()));
    }
    let metadata = [data.get("metadata"), data.get("litellm_metadata")]
        .into_iter()
        .flatten()
        .find(|value| value.is_truthy())?
        .as_object()?;
    let hint = non_empty_text(metadata, "user_api_key_user_id")
        .or_else(|| non_empty_text(metadata, &base.user_id_field))?;
    Some(ResolvedUserId::Untrusted(hint.to_owned()))
}

// Logging hooks trust only the id the proxy injected after it stripped caller-supplied user_api_key_* keys.
#[must_use]
pub fn resolve_logging_user_id(kwargs: &Object) -> Option<String> {
    let injected = kwargs
        .get("litellm_params")
        .and_then(Json::as_object)
        .and_then(|params| params.get("metadata"))
        .and_then(Json::as_object)
        .and_then(|metadata| non_empty_text(metadata, "user_api_key_user_id"));
    injected
        .or_else(|| non_empty_text(kwargs, "user_api_key_user_id"))
        .map(str::to_owned)
}

mod tests;
