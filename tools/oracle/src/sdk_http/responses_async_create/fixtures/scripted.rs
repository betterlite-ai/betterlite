use async_trait::async_trait;
use betterlite_core::api::response_id_traversal::response_id::transcode;
use betterlite_core::api::responses_create_call::deps::AsyncPromptPort;
use betterlite_core::api::responses_json::{decision_json, json_value};
use betterlite_core::api::responses_parameters::prompt_merge::ReturnedMessage;
use betterlite_core::api::responses_provider_selection::ParameterMap;
use betterlite_core::api::responses_provider_selection::prompting::{
    CARRY_UNMATCHED_MESSAGE_POINTS, HookRequest, MergeAnswer, MergeRequest, PromptManagerError,
};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

use super::{HookCall, HookObservation};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HookScript {
    answer: HookAnswer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookAnswer {
    model: String,
    messages: Vec<Json>,
    optional_params: Json,
}

// The scripted stand-in for LiteLLMLoggingObj's async prompt hook, counted the way the adapter observes it
pub(super) struct ScriptedPort {
    answer: Option<HookAnswer>,
    checks: AtomicU32,
    ran: AtomicU32,
    records: Mutex<Vec<HookCall>>,
}

impl ScriptedPort {
    pub(super) fn new(script: Option<HookScript>) -> Self {
        Self {
            answer: script.map(|hook| hook.answer),
            checks: AtomicU32::new(0),
            ran: AtomicU32::new(0),
            records: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn observation(&self) -> HookObservation {
        HookObservation {
            checks: self.checks.load(Ordering::SeqCst),
            ran: self.ran.load(Ordering::SeqCst),
            sync_checks: 0,
            calls: self
                .records
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }
}

#[async_trait]
impl AsyncPromptPort for ScriptedPort {
    fn runs_prompt_hooks(&self, request: &HookRequest<'_>) -> bool {
        let runs = prompt_management_runs(request);
        self.checks.fetch_add(1, Ordering::SeqCst);
        if runs {
            self.ran.fetch_add(1, Ordering::SeqCst);
        }
        runs
    }

    async fn async_merge_prompt(
        &self,
        request: &MergeRequest<'_>,
    ) -> Result<MergeAnswer, PromptManagerError> {
        let Some(answer) = &self.answer else {
            return Err(PromptManagerError {
                message: String::from("the case configured no hook answer"),
            });
        };
        let call = observed(request);
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        let messages = answer
            .messages
            .iter()
            .map(|message| ReturnedMessage {
                source: None,
                message: json_value(message),
            })
            .collect();
        let optional_params = match &answer.optional_params {
            Json::Null => ParameterMap::new(),
            value @ (Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_)) => transcode(value).map_err(|error| PromptManagerError {
                message: error.to_string(),
            })?,
        };
        Ok(MergeAnswer {
            model: answer.model.clone(),
            messages,
            optional_params,
        })
    }
}

// The hook call as the case observes it: what aresponses passed and what the port answered under
fn observed(request: &MergeRequest<'_>) -> HookCall {
    HookCall {
        model: request.model.to_owned(),
        prompt_id: request.prompt_id.map(str::to_owned),
        prompt_label: request.prompt_label.map(str::to_owned),
        prompt_version: request.prompt_version.map(decision_json),
        prompt_variables: request.prompt_variables.map(decision_json),
        messages: Json::List(
            request
                .messages
                .iter()
                .map(|message| decision_json(&message.message))
                .collect(),
        ),
        carry: request
            .request_kwargs
            .contains_key(CARRY_UNMATCHED_MESSAGE_POINTS),
        aresponses: truthy(request.non_default_params.get("aresponses")),
    }
}

// LiteLLM's Logging.should_run_prompt_management_hooks, for the params aresponses passes it
fn prompt_management_runs(request: &HookRequest<'_>) -> bool {
    if request
        .prompt_id
        .is_some_and(|prompt_id| !prompt_id.is_empty())
    {
        return true;
    }
    [
        "cache_control_injection_points",
        "knowledge_bases",
        "vector_store_ids",
    ]
    .iter()
    .any(|name| truthy(request.non_default_params.get(*name)))
}

fn truthy(value: Option<&JsonValue>) -> bool {
    match value {
        None | Some(JsonValue::Null) => false,
        Some(JsonValue::Bool(flag)) => *flag,
        Some(JsonValue::Number(number)) => number.as_f64().is_some_and(|float| float != 0.0_f64),
        Some(JsonValue::Text(text)) => !text.is_empty(),
        Some(JsonValue::Array(items)) => !items.is_empty(),
        Some(JsonValue::Object(fields)) => !fields.is_empty(),
    }
}
