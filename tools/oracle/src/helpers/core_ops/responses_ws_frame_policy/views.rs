use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::api::responses_stream_events::ProjectQuotaCallback;
use betterlite_core::api::responses_stream_primitives::error_fields::{
    StreamErrorAttributes, StreamErrorObject, error_status_and_message,
};
use betterlite_core::api::responses_ws_frame_policy::events::{
    wrap_item_encrypted_content, ws_event_error,
};
use betterlite_core::api::responses_ws_frame_policy::quota::enforce_or_reject_frame;
use betterlite_core::api::responses_ws_frame_policy::{
    DELTA_EVENT_TYPES, FAILURE_EVENT_TYPES, FramePolicy, LOGGED_EVENT_TYPES,
    MASKABLE_TEXT_BLOCK_TYPES, OUTPUT_DONE_EVENT_TYPES, OUTPUT_ITEM_EVENT_TYPES,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use super::fakes::{Calls, Guard, QUOTA_MESSAGE, Quota, Socket, Verdict};

#[derive(Deserialize)]
struct Rejection {
    #[serde(rename = "type")]
    kind: String,
    error: RejectionError,
}

#[derive(Deserialize)]
struct RejectionError {
    #[serde(rename = "type")]
    kind: String,
    message: String,
}

pub(super) async fn quota_view(
    message: &str,
    authorized_model: Option<&str>,
    verdicts: &[Verdict],
    refuse: bool,
) -> HelperResult {
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let callbacks: Vec<Arc<dyn ProjectQuotaCallback>> = verdicts
        .iter()
        .enumerate()
        .map(|(index, verdict)| -> Arc<dyn ProjectQuotaCallback> {
            Arc::new(Quota {
                index,
                verdict: *verdict,
                calls: Arc::clone(&calls),
            })
        })
        .collect();
    let fields = BTreeMap::from([("user_id".to_owned(), JsonData::String("u-1".to_owned()))]);
    let user = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
    let socket = Socket {
        refuse,
        sent: Mutex::new(Vec::new()),
    };
    let policy = FramePolicy::<Guard> {
        quota_callbacks: &callbacks,
        user_api_key_dict: Some(&user),
        guardrail_callbacks: &[],
        output_guardrail_callbacks: &[],
        authorized_model,
        request_defaults: None,
    };
    let result = enforce_or_reject_frame(&policy, &socket, message)
        .await
        .map_or_else(|_error| json!("error"), |accepted| json!(accepted));
    let recorded = calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok(json!({"result": result, "calls": recorded, "frames": rejection_frames(&socket)?}))
}

// LiteLLM sends str(error), so each side reports whether the frame carries its own error text
fn rejection_frames(socket: &Socket) -> Result<Vec<Value>, Box<HelperError>> {
    let mut frames = Vec::new();
    for text in socket
        .sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
    {
        let frame: Rejection = serde_json::from_str(text).or_value_error()?;
        frames.push(json!({
            "type": frame.kind,
            "error_type": frame.error.kind,
            "message_is_error_text": frame.error.message == QUOTA_MESSAGE,
        }));
    }
    Ok(frames)
}

// LiteLLM maps the first failure event's error to an exception; its status is what a case can see
pub(super) fn failure_status(event: &IndexMap<String, Json>) -> Value {
    let failed = matches!(event.get("type"), Some(Json::Text(kind)) if FAILURE_EVENT_TYPES.contains(&kind.as_str()));
    if !failed {
        return Value::Null;
    }
    let object = if let Some(Json::Object(fields)) = ws_event_error(event) {
        StreamErrorObject::Mapping(fields)
    } else {
        StreamErrorObject::Attributes(StreamErrorAttributes::default())
    };
    json!(error_status_and_message(object).0)
}

// the relay wraps only added and done items whose event has no response object
pub(super) fn wrap_view(
    event: &IndexMap<String, Json>,
    litellm_metadata: &Json,
) -> Option<IndexMap<String, Json>> {
    let empty = IndexMap::new();
    let metadata = if let Json::Object(fields) = litellm_metadata {
        fields
    } else {
        &empty
    };
    let item_event = matches!(event.get("type"), Some(Json::Text(kind)) if OUTPUT_ITEM_EVENT_TYPES.contains(&kind.as_str()));
    let response_event = matches!(event.get("response"), Some(Json::Object(_)));
    if response_event || !item_event {
        return None;
    }
    wrap_item_encrypted_content(event, metadata)
}

pub(super) fn sets_view(candidates: &[String]) -> Value {
    let mut maskable = MASKABLE_TEXT_BLOCK_TYPES.to_vec();
    maskable.sort_unstable();
    let mut deltas = DELTA_EVENT_TYPES.to_vec();
    deltas.sort_unstable();
    let mut done = OUTPUT_DONE_EVENT_TYPES.to_vec();
    done.sort_unstable();
    let failure: Vec<&String> = candidates
        .iter()
        .filter(|kind| FAILURE_EVENT_TYPES.contains(&kind.as_str()))
        .collect();
    let output_item: Vec<&String> = candidates
        .iter()
        .filter(|kind| OUTPUT_ITEM_EVENT_TYPES.contains(&kind.as_str()))
        .collect();
    json!({
        "logged": LOGGED_EVENT_TYPES,
        "maskable": maskable,
        "failure": failure,
        "output_item": output_item,
        "delta": deltas,
        "done": done,
    })
}
