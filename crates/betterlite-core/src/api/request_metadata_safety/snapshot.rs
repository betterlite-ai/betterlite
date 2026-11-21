use std::sync::Arc;

use betterlite_logging::callback_logger::event::GuardrailRequestSnapshot;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::fields::{
    TRANSPORT_ONLY_CREDENTIAL_KEYS, TRUSTED_CALLBACK_VARS_FIELD, is_callback_dynamic_param,
};
use crate::api::request_metadata_safety::metadata::metadata_name_from_data;

const PROXY_SERVER_REQUEST_KEY: &str = "proxy_server_request";
const BODY_KEY: &str = "body";
const BODY_SNAPSHOT_EXCLUDED_KEYS: [&str; 3] = [
    "secret_fields",
    "proxy_server_request",
    "litellm_logging_obj",
];

// ast-grep-ignore: no-long-names
pub fn refresh_proxy_server_request_body_snapshot(
    data: &mut RequestData,
    mut logging: Option<&mut LoggingState>,
    guardrails_applied: bool,
) {
    // rebuild the audit body from the current request and, after a request-mutating guardrail, keep its snapshot
    if let Some(state) = logging.as_deref_mut() {
        state.shadow_eval_request_snapshot = None;
    }
    if !matches!(data.get(PROXY_SERVER_REQUEST_KEY), Some(Json::Object(_))) {
        return;
    }
    let body: RequestData = data
        .iter()
        .filter(|(name, _value)| !is_body_snapshot_excluded(name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let metadata_field = metadata_name_from_data(data);
    let metadata = match data.get(metadata_field.as_str()) {
        Some(Json::Object(values)) => values.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => IndexMap::new(),
    };
    if let Some(Json::Object(request)) = data.get_mut(PROXY_SERVER_REQUEST_KEY) {
        request.insert(BODY_KEY.to_owned(), Json::Object(body.clone()));
    }
    if !guardrails_applied {
        return;
    }
    if let Some(state) = logging {
        state.shadow_eval_request_snapshot =
            GuardrailRequestSnapshot::capture(&body, &metadata).map(Arc::new);
    }
}

fn is_body_snapshot_excluded(name: &str) -> bool {
    BODY_SNAPSHOT_EXCLUDED_KEYS.contains(&name)
        || TRANSPORT_ONLY_CREDENTIAL_KEYS.contains(&name)
        || is_callback_dynamic_param(name)
        || name == TRUSTED_CALLBACK_VARS_FIELD
}
