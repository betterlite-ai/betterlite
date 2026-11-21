mod fakes;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::error::Error;
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
use betterlite_core::api::responses_ws_frame_policy::request::mask_response_create;
use betterlite_core::api::responses_ws_frame_policy::response::{
    mask_response_completed, unmask_response_event,
};
use betterlite_core::api::responses_ws_frame_policy::{
    DELTA_EVENT_TYPES, FAILURE_EVENT_TYPES, FramePolicy, FramePolicyError, InputGuardrail,
    LOGGED_EVENT_TYPES, MASKABLE_TEXT_BLOCK_TYPES, OUTPUT_DONE_EVENT_TYPES,
    OUTPUT_ITEM_EVENT_TYPES, RequestData, UnmaskPiiText,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::WsRequestDefaults;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::to_raw_value;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::fakes::{Calls, Guard, QUOTA_MESSAGE, Quota, Socket, Verdict, recording_unmask};

type Replacements = Vec<IndexMap<String, String>>;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Quota {
        message: String,
        authorized_model: Option<String>,
        verdicts: Vec<Verdict>,
        refuse_send: bool,
    },
    Create {
        message: String,
        authorized_model: Option<String>,
        defaults: Option<WsRequestDefaults<Json>>,
        guardrails: Replacements,
        request_data: IndexMap<String, Json>,
    },
    Unmask {
        message: String,
        guardrails: usize,
        request_data: IndexMap<String, Json>,
    },
    Completed {
        message: String,
        guardrails: Replacements,
        request_data: IndexMap<String, Json>,
    },
    Failure {
        events: Vec<IndexMap<String, Json>>,
    },
    Wrap {
        litellm_metadata: Json,
        event: IndexMap<String, Json>,
    },
    Sets {
        candidates: Vec<String>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

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

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let mut answers = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            answers.push(runtime.block_on(answer(step))?);
        }
        Ok(Value::Array(answers))
    }
}

async fn answer(step: &Step) -> HelperResult {
    match step {
        Step::Quota {
            message,
            authorized_model,
            verdicts,
            refuse_send,
        } => quota_view(message, authorized_model.as_deref(), verdicts, *refuse_send).await,
        Step::Create {
            message,
            authorized_model,
            defaults,
            guardrails,
            request_data,
        } => {
            let calls: Calls = Arc::new(Mutex::new(Vec::new()));
            let echo = |text: &str, _pii_tokens: &IndexMap<String, String>| text.to_owned();
            let inputs = guards(guardrails, &calls, &echo);
            let policy = FramePolicy {
                quota_callbacks: &[],
                user_api_key_dict: None,
                guardrail_callbacks: &inputs,
                output_guardrail_callbacks: &[],
                authorized_model: authorized_model.as_deref(),
                request_defaults: defaults.as_ref(),
            };
            let mut data = stored(request_data)?;
            let outcome = mask_response_create(&policy, &mut data, message).await;
            Ok(frame_view(outcome, &calls, &data))
        }
        Step::Unmask {
            message,
            guardrails,
            request_data,
        } => unmask_view(message, *guardrails, request_data),
        Step::Completed {
            message,
            guardrails,
            request_data,
        } => completed_view(message, guardrails, request_data).await,
        Step::Failure { events } => Ok(Value::Array(events.iter().map(failure_status).collect())),
        Step::Wrap {
            litellm_metadata,
            event,
        } => Ok(json!(wrap_view(event, litellm_metadata))),
        Step::Sets { candidates } => Ok(sets_view(candidates)),
    }
}

fn unmask_view(
    message: &str,
    guardrails: usize,
    request_data: &IndexMap<String, Json>,
) -> HelperResult {
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let unmaskers: Vec<Box<UnmaskPiiText>> = (0..guardrails)
        .map(|index| -> Box<UnmaskPiiText> { Box::new(recording_unmask(index, &calls)) })
        .collect();
    let inputs: Vec<InputGuardrail<'_, Guard>> = unmaskers
        .iter()
        .enumerate()
        .map(|(index, unmask)| InputGuardrail {
            callback: Guard {
                index,
                replacements: IndexMap::new(),
                calls: Arc::clone(&calls),
            },
            unmask_pii_text: unmask.as_ref(),
        })
        .collect();
    let policy = FramePolicy {
        quota_callbacks: &[],
        user_api_key_dict: None,
        guardrail_callbacks: &inputs,
        output_guardrail_callbacks: &[],
        authorized_model: None,
        request_defaults: None,
    };
    let data = stored(request_data)?;
    let outcome = unmask_response_event(&policy, &data, message);
    Ok(frame_view(outcome, &calls, &data))
}

async fn completed_view(
    message: &str,
    guardrails: &Replacements,
    request_data: &IndexMap<String, Json>,
) -> HelperResult {
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let echo = |text: &str, _pii_tokens: &IndexMap<String, String>| text.to_owned();
    let outputs: Vec<Guard> = guards(guardrails, &calls, &echo)
        .into_iter()
        .map(|guardrail| guardrail.callback)
        .collect();
    let policy = FramePolicy {
        quota_callbacks: &[],
        user_api_key_dict: None,
        guardrail_callbacks: &[],
        output_guardrail_callbacks: &outputs,
        authorized_model: None,
        request_defaults: None,
    };
    let data = stored(request_data)?;
    let outcome = mask_response_completed(&policy, &data, message).await;
    Ok(frame_view(outcome, &calls, &data))
}

async fn quota_view(
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

fn guards<'policy>(
    replacements: &Replacements,
    calls: &Calls,
    unmask: &'policy UnmaskPiiText,
) -> Vec<InputGuardrail<'policy, Guard>> {
    replacements
        .iter()
        .enumerate()
        .map(|(index, pairs)| InputGuardrail {
            callback: Guard {
                index,
                replacements: pairs.clone(),
                calls: Arc::clone(calls),
            },
            unmask_pii_text: unmask,
        })
        .collect()
}

fn stored(request_data: &IndexMap<String, Json>) -> Result<RequestData, Box<HelperError>> {
    let mut data = RequestData::new();
    for (key, value) in request_data {
        let raw = to_raw_value(value).or_value_error()?;
        data.insert(key.clone(), raw);
    }
    Ok(data)
}

// the Python side reports whether LiteLLM handed back the very string it was given
fn frame_view<GuardError>(
    outcome: Result<Cow<'_, str>, FramePolicyError<GuardError>>,
    calls: &Calls,
    data: &RequestData,
) -> Value
where
    GuardError: Error + 'static,
{
    let (error, same, frame) = match outcome {
        Ok(Cow::Borrowed(_)) => (false, true, Value::Null),
        Ok(Cow::Owned(text)) => (
            false,
            false,
            serde_json::from_str(&text).unwrap_or(Value::Null),
        ),
        Err(_) => (true, false, Value::Null),
    };
    let metadata = data
        .get("metadata")
        .and_then(|raw| serde_json::from_str::<Value>(raw.get()).ok());
    let recorded = calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    json!({"error": error, "same": same, "frame": frame, "calls": recorded, "metadata": metadata})
}

// LiteLLM maps the first failure event's error to an exception; its status is what a case can see
fn failure_status(event: &IndexMap<String, Json>) -> Value {
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
fn wrap_view(
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

fn sets_view(candidates: &[String]) -> Value {
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
