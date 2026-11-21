use betterlite_config::sections::request_metadata_safety::MissingSessionIdPolicy;
use betterlite_models::decisions::Json;

use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::metadata::MetadataField;
use crate::api::request_setup_options::error::RequestSetupOptionsError;
use crate::api::request_setup_options::{SessionIdSource, SetupDiagnostic};

const SESSION_ID_KEY: &str = "session_id";
const TRACE_ID_KEY: &str = "trace_id";
const BODY_SESSION_ID_KEY: &str = "litellm_session_id";
const BODY_TRACE_ID_KEY: &str = "litellm_trace_id";
const GENERATED_KEY: &str = "litellm_session_id_generated";

// the generate, reject and unknown stages of apply_missing_session_id_policy; B1105's stage has already handled omit
pub fn missing_session_stage<Ids>(
    data: &mut RequestData,
    field: MetadataField,
    policy: Option<&MissingSessionIdPolicy>,
    is_inference_route: bool,
    session_ids: &Ids,
) -> Result<Option<SetupDiagnostic>, RequestSetupOptionsError>
where
    Ids: SessionIdSource,
{
    let Some(resolved) = policy else {
        return Ok(None);
    };
    let Some(Json::Object(metadata)) = data.get(field.as_str()) else {
        return Ok(None);
    };
    if !is_inference_route
        || !matches!(data.get(BODY_SESSION_ID_KEY), None | Some(Json::Null))
        || !matches!(metadata.get(SESSION_ID_KEY), None | Some(Json::Null))
    {
        return Ok(None);
    }
    match resolved {
        MissingSessionIdPolicy::Omit => Ok(None),
        MissingSessionIdPolicy::Generate => {
            generate_session_id(data, field, session_ids);
            Ok(None)
        }
        MissingSessionIdPolicy::Reject => Err(RequestSetupOptionsError::SessionRequired),
        MissingSessionIdPolicy::Unrecognized(name) => {
            tracing::warn!(
                "Ignoring unknown general_settings.missing_session_id={name:?}; expected 'generate', 'reject' or 'omit'"
            );
            Ok(Some(SetupDiagnostic::UnknownSessionPolicy {
                policy: name.clone(),
            }))
        }
    }
}

// a trace id the request already carries becomes its session id, so the trace and the session stay one
fn generate_session_id<Ids>(data: &mut RequestData, field: MetadataField, session_ids: &Ids)
where
    Ids: SessionIdSource,
{
    let metadata_trace_id = match data.get(field.as_str()) {
        Some(Json::Object(metadata)) => metadata.get(TRACE_ID_KEY),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    };
    let session_id = [data.get(BODY_TRACE_ID_KEY), metadata_trace_id]
        .into_iter()
        .find_map(|candidate| {
            if let Some(Json::Text(text)) = candidate {
                Some(text.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| session_ids.session_id());
    data.insert(
        BODY_SESSION_ID_KEY.to_owned(),
        Json::Text(session_id.clone()),
    );
    if matches!(data.get(BODY_TRACE_ID_KEY), None | Some(Json::Null)) {
        data.insert(BODY_TRACE_ID_KEY.to_owned(), Json::Text(session_id.clone()));
    }
    if let Some(Json::Object(metadata)) = data.get_mut(field.as_str()) {
        metadata.insert(SESSION_ID_KEY.to_owned(), Json::Text(session_id));
        metadata.insert(GENERATED_KEY.to_owned(), Json::Bool(true));
    }
}
