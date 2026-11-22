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

use self::fakes::{Calls, Guard, Verdict, recording_unmask};
use self::views::{failure_status, quota_view, sets_view, wrap_view};

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
        Step::Create { .. } => create_view(step).await,
        Step::Unmask { .. } => unmask_view(step),
        Step::Completed { .. } => completed_view(step).await,
        Step::Failure { events } => Ok(Value::Array(events.iter().map(failure_status).collect())),
        Step::Wrap {
            litellm_metadata,
            event,
        } => Ok(json!(wrap_view(event, litellm_metadata))),
        Step::Sets { candidates } => Ok(sets_view(candidates)),
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
    let Step::Create {
        message,
        authorized_model,
        defaults,
        guardrails,
        request_data,
    } = step
    else {
        return Err(wrong_step("create"));
    };
    let mut probe = Probe::start(request_data)?;
    let echo = |text: &str, _pii_tokens: &IndexMap<String, String>| text.to_owned();
    let inputs = guards(guardrails, &probe.calls, &echo);
    let outcome = mask_response_create(
        &frame_policy(&inputs, &[], authorized_model.as_deref(), defaults.as_ref()),
        &mut probe.data,
        message,
    )
    .await;
    Ok(probe.report(outcome))
}

fn unmask_view(step: &Step) -> HelperResult {
    let Step::Unmask {
        message,
        guardrails,
        request_data,
    } = step
    else {
        return Err(wrong_step("unmask"));
    };
    let probe = Probe::start(request_data)?;
    let mut unmaskers: Vec<Box<UnmaskPiiText>> = Vec::with_capacity(*guardrails);
    for index in 0..*guardrails {
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
    let outcome = unmask_response_event(&policy, &probe.data, message);
    Ok(probe.report(outcome))
}

async fn completed_view(step: &Step) -> HelperResult {
    let Step::Completed {
        message,
        guardrails,
        request_data,
    } = step
    else {
        return Err(wrong_step("completed"));
    };
    let probe = Probe::start(request_data)?;
    let mut outputs = Vec::with_capacity(guardrails.len());
    for (index, replacements) in guardrails.iter().enumerate() {
        outputs.push(Guard {
            index,
            replacements: replacements.clone(),
            calls: Arc::clone(&probe.calls),
        });
    }
    let policy = frame_policy(&[], &outputs, None, None);
    let outcome = mask_response_completed(&policy, &probe.data, message).await;
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
