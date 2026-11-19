use betterlite_logging::spend_metadata::{
    SpendLogStatus, looks_like_model_name, optional_object_field, optional_text_field,
    spend_logs_id, status_for_spend_log,
};
use betterlite_logging::spend_sanitize::UNKNOWN_MODEL;
use betterlite_logging::spend_sanitize::raw_model::scrub_raw_model_from_errors;
use betterlite_logging::spend_sanitize::value::SpendValue;
use betterlite_models::call_helpers::model_name::reconstruct_model_name;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::source::{AttributionField, sl_attribution_fallback};
use super::{Context, DEFAULT_CALL_TYPE, SpendPayloadError, SpendResponse};
use crate::spend::attribution::{catalog_group_provider, configured_model_group};

// The provenance the record keeps: the request id before its cache-hit suffix, the model spellings and the call id.
pub(crate) struct Attribution {
    pub(crate) base_id: Option<String>,
    pub(crate) model_name: String,
    pub(crate) model_group: String,
    pub(crate) model_id: String,
    pub(crate) api_base: String,
    pub(crate) provider: Option<String>,
    pub(crate) call_id: Option<String>,
    pub(crate) raw_model: String,
    pub(crate) placeholdered: bool,
    pub(crate) persisted_metadata: JsonObject,
}

pub(crate) fn resolve(context: &Context<'_>) -> Result<Attribution, SpendPayloadError> {
    let raw = raw_model(context.kwargs);
    let model_id = model_id(context)?;
    let group = model_group(context)?;
    let api_base = api_base(context)?;
    let logged = logged_provider(context)?;
    let provider = match logged.clone() {
        Some(name) => Some(name),
        None => grouped_provider(context, &group)?,
    };
    let resolved = resolved_model(context, raw.text, logged.as_deref())?;
    let unknown = matches!(context.request.response, SpendResponse::UnknownModelError)
        || raw.malformed
        || prompt_shaped_failure(context, &model_id, &group, &resolved);
    let model_name = if unknown {
        UNKNOWN_MODEL
    } else {
        resolved.as_str()
    };
    let placeholdered = model_name == UNKNOWN_MODEL;
    let persisted_metadata = match placeholdered {
        true => scrub_metadata(context, raw.text)?,
        false => context.metadata.clone(),
    };
    Ok(Attribution {
        base_id: base_id(context),
        model_name: model_name.to_owned(),
        model_group: persisted_group(placeholdered, &group, raw.text),
        model_id,
        api_base,
        provider,
        call_id: call_id(context)?,
        raw_model: raw.text.to_owned(),
        placeholdered,
        persisted_metadata,
    })
}

struct RawModel<'request> {
    text: &'request str,
    malformed: bool,
}

// kwargs.get("model"): only a string is a model name, and any other present value is the malformed case.
fn raw_model(kwargs: &JsonObject) -> RawModel<'_> {
    match kwargs.get("model") {
        Some(Json::Text(text)) => RawModel {
            text,
            malformed: false,
        },
        Some(Json::Null) | None => RawModel {
            text: "",
            malformed: false,
        },
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => RawModel {
            text: "",
            malformed: true,
        },
    }
}

// get_spend_logs_id over the response mapping the caller held: a non-mapping response carries no id key.
fn base_id(context: &Context<'_>) -> Option<String> {
    let absent = JsonObject::new();
    let response = match context.request.response {
        SpendResponse::Mapping(fields) => fields,
        SpendResponse::Absent | SpendResponse::Other | SpendResponse::UnknownModelError => &absent,
    };
    spend_logs_id(
        context.call_type.unwrap_or(DEFAULT_CALL_TYPE),
        response,
        context.kwargs,
    )
}

// metadata.model_info.id or the standard payload's model_id, and an empty id counts as absent.
fn model_id(context: &Context<'_>) -> Result<String, SpendPayloadError> {
    let info = optional_object_field(context.metadata.get("model_info"), "model_info")?;
    let stored = match info {
        Some(fields) => optional_text_field(fields.get("id"), "model_info.id")?,
        None => None,
    };
    Ok(stored.filter(|id| !id.is_empty()).map_or_else(
        || standard_text(context, AttributionField::ModelId),
        ToOwned::to_owned,
    ))
}

fn model_group(context: &Context<'_>) -> Result<String, SpendPayloadError> {
    let stored = optional_text_field(context.metadata.get("model_group"), "model_group")?;
    Ok(stored.filter(|group| !group.is_empty()).map_or_else(
        || standard_text(context, AttributionField::ModelGroup),
        ToOwned::to_owned,
    ))
}

fn api_base(context: &Context<'_>) -> Result<String, SpendPayloadError> {
    let stored = optional_text_field(context.litellm_params.get("api_base"), "api_base")?;
    Ok(stored.filter(|base| !base.is_empty()).map_or_else(
        || standard_text(context, AttributionField::ApiBase),
        ToOwned::to_owned,
    ))
}

fn standard_text(context: &Context<'_>, field: AttributionField) -> String {
    sl_attribution_fallback(context.standard.as_ref(), field)
}

// kwargs.custom_llm_provider, then the standard payload's provider, with an empty string reading as absent.
fn logged_provider(context: &Context<'_>) -> Result<Option<String>, SpendPayloadError> {
    let explicit = optional_text_field(
        context.kwargs.get("custom_llm_provider"),
        "custom_llm_provider",
    )?;
    if let Some(name) = explicit.filter(|name| !name.is_empty()) {
        return Ok(Some(name.to_owned()));
    }
    let fallback = standard_text(context, AttributionField::CustomLlmProvider);
    Ok((!fallback.is_empty()).then_some(fallback))
}

// _model_group_provider runs only when neither the kwargs nor the standard payload named a provider.
fn grouped_provider(
    context: &Context<'_>,
    group: &str,
) -> Result<Option<String>, SpendPayloadError> {
    let Some(router) = context.request.router else {
        return Ok(None);
    };
    let provider =
        catalog_group_provider(group, router.catalog, router.settings, router.resolution)?;
    Ok(provider)
}

// A nonempty standard payload model wins, and the reconstructed name keeps the deployment's own prefix.
fn resolved_model(
    context: &Context<'_>,
    raw: &str,
    logged: Option<&str>,
) -> Result<String, SpendPayloadError> {
    let logged_model = context
        .standard
        .as_ref()
        .and_then(|source| source.model.as_ref())
        .filter(|model| !model.is_empty());
    if let Some(model) = logged_model {
        return Ok(model.clone());
    }
    let deployment = optional_text_field(context.metadata.get("deployment"), "deployment")?;
    Ok(reconstruct_model_name(raw, logged, deployment))
}

// A failed request whose model reads like a prompt hides it unless the deployment or group names it.
fn prompt_shaped_failure(
    context: &Context<'_>,
    model_id: &str,
    group: &str,
    resolved: &str,
) -> bool {
    status_for_spend_log(&context.metadata) == SpendLogStatus::Failure
        && model_id.is_empty()
        && !looks_like_model_name(resolved)
        && !configured_group(context, group)
}

fn configured_group(context: &Context<'_>, group: &str) -> bool {
    context.request.router.is_some_and(|router| {
        configured_model_group(group, router.recognized_groups, router.team_public_names)
    })
}

// The group survives unless it is the unsafe raw model itself.
fn persisted_group(placeholdered: bool, group: &str, raw: &str) -> String {
    let cleared = placeholdered && group == raw && !looks_like_model_name(raw);
    match cleared {
        true => String::new(),
        false => group.to_owned(),
    }
}

fn call_id(context: &Context<'_>) -> Result<Option<String>, SpendPayloadError> {
    let kwarg = optional_text_field(context.kwargs.get("litellm_call_id"), "litellm_call_id")?;
    match kwarg.filter(|id| !id.is_empty()) {
        Some(id) => Ok(Some(id.to_owned())),
        None => Ok(optional_text_field(
            context.litellm_params.get("litellm_call_id"),
            "litellm_call_id",
        )?
        .map(ToOwned::to_owned)),
    }
}

// The placeholdered record still carries error_information, with every raw model spelling replaced.
fn scrub_metadata(context: &Context<'_>, raw_model: &str) -> Result<JsonObject, SpendPayloadError> {
    let mut fields = context.metadata.clone();
    let scrubbed = scrub_error_information(fields.get("error_information"), raw_model)?;
    fields.insert("error_information".to_owned(), scrubbed);
    Ok(fields)
}

fn scrub_error_information(
    information: Option<&Json>,
    raw_model: &str,
) -> Result<Json, SpendPayloadError> {
    match information {
        None | Some(Json::Null) => Ok(Json::Null),
        Some(stored @ Json::Object(_)) => {
            let SpendValue::Mapping(mapping) = SpendValue::from(stored) else {
                return Ok(stored.clone());
            };
            Ok(scrub_raw_model_from_errors(&mapping, raw_model))
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(SpendPayloadError::FieldType {
                field: "error_information",
            })
        }
    }
}
