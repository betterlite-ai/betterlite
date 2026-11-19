use betterlite_logging::end_user::CostTrackingService;
// ast-grep-ignore: no-long-names
use betterlite_logging::end_user::get_end_user_id_for_cost_tracking;
use betterlite_logging::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use betterlite_logging::spend_metadata::optional_text_field;
use betterlite_logging::spend_metadata::redaction::{
    is_non_secret_key_value, redact_logged_api_key,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::{Context, SpendPayloadError, SpendPayloadSettings};

// The identity fields the record keeps as plain text: api_key is redacted, the rest carry the source's "" default.
pub(crate) struct Envelope {
    pub(crate) api_key: String,
    pub(crate) user: String,
    pub(crate) team_id: String,
    pub(crate) organization_id: String,
    pub(crate) end_user: String,
    pub(crate) request_tags: String,
}

pub(crate) fn resolve(
    context: &Context<'_>,
    settings: &SpendPayloadSettings,
) -> Result<Envelope, SpendPayloadError> {
    Ok(Envelope {
        api_key: api_key(context)?,
        user: text_or_empty(&context.metadata, "user_api_key_user_id")?,
        team_id: text_or_empty(&context.metadata, "user_api_key_team_id")?,
        organization_id: text_or_empty(&context.metadata, "user_api_key_org_id")?,
        end_user: end_user(context, settings)?,
        request_tags: request_tags(context)?,
    })
}

// The merged metadata key is hashed unless the trusted hash proves it already is a safe value.
fn api_key(context: &Context<'_>) -> Result<String, SpendPayloadError> {
    let raw_key = optional_text_field(context.metadata.get("user_api_key"), "user_api_key")?;
    let trusted = optional_text_field(
        context.metadata.get("user_api_key_hash"),
        "user_api_key_hash",
    )?;
    let alias = optional_text_field(
        context.metadata.get("user_api_key_alias"),
        "user_api_key_alias",
    )?;
    let already_redacted =
        trusted.is_some_and(|hash| is_non_secret_key_value(hash, alias) && raw_key == Some(hash));
    let redacted = redact_logged_api_key(raw_key, already_redacted, alias).unwrap_or_default();
    if !redacted.is_empty() {
        return Ok(redacted);
    }
    Ok(standard_api_key(context))
}

fn standard_api_key(context: &Context<'_>) -> String {
    let stored = context
        .standard
        .as_ref()
        .and_then(|source| source.metadata.as_ref());
    let hash = stored.and_then(|fields| fields.user_api_key_hash.as_deref());
    let alias = stored.and_then(|fields| fields.user_api_key_alias.as_deref());
    redact_logged_api_key(hash, true, alias).unwrap_or_default()
}

// get_end_user_id_for_cost_tracking's answer falls back to the standard payload's end user, then to "".
fn end_user(
    context: &Context<'_>,
    settings: &SpendPayloadSettings,
) -> Result<String, SpendPayloadError> {
    // ast-grep-ignore: no-long-names
    let resolved = get_end_user_id_for_cost_tracking(
        &context.litellm_params,
        CostTrackingService::LitellmLogging,
        &settings.end_user,
    )?;
    let stored = context
        .standard
        .as_ref()
        .and_then(|source| source.metadata.as_ref())
        .and_then(|fields| fields.user_api_key_end_user_id.clone());
    Ok(resolved
        .filter(|id| !id.is_empty())
        .or(stored)
        .unwrap_or_default())
}

fn text_or_empty(metadata: &JsonObject, field: &'static str) -> Result<String, SpendPayloadError> {
    Ok(optional_text_field(metadata.get(field), field)?
        .unwrap_or_default()
        .to_owned())
}

// A list of tags is dumped, any other metadata tags value reads as [], and a non-null standard list overrides both.
fn request_tags(context: &Context<'_>) -> Result<String, SpendPayloadError> {
    let stored = context
        .standard
        .as_ref()
        .and_then(|source| source.request_tags.as_ref());
    if let Some(tags) = stored {
        return dumps(tags, "request_tags");
    }
    match context.metadata.get("tags") {
        Some(Json::List(items)) => dumps(&Json::List(items.clone()), "tags"),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => Ok("[]".to_owned()),
    }
}

fn dumps(value: &Json, record: &'static str) -> Result<String, SpendPayloadError> {
    safe_dumps(value, DEFAULT_MAX_RECURSE_DEPTH, None)
        .map_err(|source| SpendPayloadError::Json { record, source })
}
