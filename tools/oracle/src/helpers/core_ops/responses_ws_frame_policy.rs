mod cases;
mod fakes;
mod views;

use std::borrow::Cow;
use std::error::Error;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::responses_ws_frame_policy::request::mask_response_create;
use betterlite_core::api::responses_ws_frame_policy::response::{
    mask_response_completed, unmask_response_event,
};
use betterlite_core::api::responses_ws_frame_policy::{
    FramePolicy, FramePolicyError, InputGuardrail, RequestData, UnmaskPiiText,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::WsRequestDefaults;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::to_raw_value;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::cases::Step;
use self::fakes::{Calls, Guard, recording_unmask};
use self::views::{failure_status, quota_view, sets_view, wrap_view};

type Replacements = Vec<IndexMap<String, String>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
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
        Step::Quota(fields) => {
            quota_view(
                &fields.message,
                fields.authorized_model.as_deref(),
                &fields.verdicts,
                fields.refuse_send,
            )
            .await
        }
        Step::Create(_) => create_view(step).await,
        Step::Unmask(_) => unmask_view(step),
        Step::Completed(_) => completed_view(step).await,
        Step::Failure(fields) => Ok(Value::Array(
            fields.events.iter().map(failure_status).collect(),
        )),
        Step::Wrap(fields) => Ok(json!(wrap_view(&fields.event, &fields.litellm_metadata))),
        Step::Sets(fields) => Ok(sets_view(&fields.candidates)),
    }
}

// everything a case can see about one call: the recorded guardrail calls and the request data it left
struct Probe {
    calls: Calls,
    data: RequestData,
}

impl Probe {
    fn start(request_data: &IndexMap<String, Json>) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            data: stored(request_data)?,
        })
    }

    // the Python side reports whether LiteLLM handed back the very string it was given
    fn report<GuardError>(
        &self,
        outcome: Result<Cow<'_, str>, FramePolicyError<GuardError>>,
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
        let metadata = self
            .data
            .get("metadata")
            .and_then(|raw| serde_json::from_str::<Value>(raw.get()).ok());
        let recorded = self
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        json!({"error": error, "same": same, "frame": frame, "calls": recorded, "metadata": metadata})
    }
}

async fn create_view(step: &Step) -> HelperResult {
    let Step::Create(fields) = step else {
        return Err(wrong_step("create"));
    };
    let mut probe = Probe::start(&fields.request_data)?;
    let echo = |text: &str, _pii_tokens: &IndexMap<String, String>| text.to_owned();
    let inputs = guards(&fields.guardrails, &probe.calls, &echo);
    let outcome = mask_response_create(
        &frame_policy(
            &inputs,
            &[],
            fields.authorized_model.as_deref(),
            fields.defaults.as_ref(),
        ),
        &mut probe.data,
        &fields.message,
    )
    .await;
    Ok(probe.report(outcome))
}

fn unmask_view(step: &Step) -> HelperResult {
    let Step::Unmask(fields) = step else {
        return Err(wrong_step("unmask"));
    };
    let probe = Probe::start(&fields.request_data)?;
    let mut unmaskers: Vec<Box<UnmaskPiiText>> = Vec::with_capacity(fields.guardrails);
    for index in 0..fields.guardrails {
        unmaskers.push(Box::new(recording_unmask(index, &probe.calls)));
    }
    let inputs: Vec<InputGuardrail<'_, Guard>> = unmaskers
        .iter()
        .enumerate()
        .map(|(index, unmask)| InputGuardrail {
            callback: Guard {
                index,
                replacements: IndexMap::new(),
                calls: Arc::clone(&probe.calls),
            },
            unmask_pii_text: unmask.as_ref(),
        })
        .collect();
    let policy = frame_policy(&inputs, &[], None, None);
    let outcome = unmask_response_event(&policy, &probe.data, &fields.message);
    Ok(probe.report(outcome))
}

async fn completed_view(step: &Step) -> HelperResult {
    let Step::Completed(fields) = step else {
        return Err(wrong_step("completed"));
    };
    let probe = Probe::start(&fields.request_data)?;
    let mut outputs = Vec::with_capacity(fields.guardrails.len());
    for (index, replacements) in fields.guardrails.iter().enumerate() {
        outputs.push(Guard {
            index,
            replacements: replacements.clone(),
            calls: Arc::clone(&probe.calls),
        });
    }
    let policy = frame_policy(&[], &outputs, None, None);
    let outcome = mask_response_completed(&policy, &probe.data, &fields.message).await;
    Ok(probe.report(outcome))
}

fn wrong_step(expected: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: format!("the {expected} view ran on a different step"),
    })
}

fn frame_policy<'policy>(
    guardrails: &'policy [InputGuardrail<'policy, Guard>],
    outputs: &'policy [Guard],
    authorized_model: Option<&'policy str>,
    request_defaults: Option<&'policy WsRequestDefaults<Json>>,
) -> FramePolicy<'policy, Guard> {
    FramePolicy {
        quota_callbacks: &[],
        user_api_key_dict: None,
        guardrail_callbacks: guardrails,
        output_guardrail_callbacks: outputs,
        authorized_model,
        request_defaults,
    }
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
