use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_judge::JudgeParams;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::judge::apply::{JudgeInputs, LLMAsAJudgeGuardrail, RequestData};
use betterlite_guardrails::judge::call::{
    JudgeCompletion, JudgeCompletionRequest, JudgeCompletionResponse, JudgeReply,
};
use betterlite_guardrails::judge::config::{GuardrailEntry, LitellmParamsSource};
use betterlite_guardrails::judge::error::{JudgeBlockDetail, JudgeError};
use betterlite_guardrails::judge::initialize::{CallbackRegistry, initialize_guardrail};
use betterlite_guardrails::judge::record::JudgeGuardrailRecord;
use betterlite_guardrails::judge::verdict::EvalInformation;
use serde::Deserialize;

use crate::helpers::{HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyInput {
    guardrail: GuardrailConfig,
    inputs: JudgeInputs,
    #[serde(default)]
    request_data: RequestData,
    input_type: GuardrailInputType,
    #[serde(default)]
    judge: JudgeOutcome,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardrailConfig {
    guardrail_name: String,
    litellm_params: BTreeMap<String, GuardrailValue>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct JudgeOutcome {
    content: Option<String>,
    error: Option<String>,
}

struct RecordingCompletion {
    outcome: JudgeOutcome,
    request: Arc<Mutex<Option<JudgeCompletionRequest>>>,
}

#[async_trait]
impl JudgeCompletion for RecordingCompletion {
    type Error = io::Error;

    async fn complete(
        &self,
        request: &JudgeCompletionRequest,
    ) -> Result<JudgeCompletionResponse, Self::Error> {
        *self.request.lock().unwrap_or_else(PoisonError::into_inner) = Some(request.clone());
        if let Some(message) = &self.outcome.error {
            return Err(io::Error::other(message.clone()));
        }
        Ok(JudgeCompletionResponse {
            choices: vec![JudgeReply {
                content: self.outcome.content.clone(),
            }],
        })
    }
}

struct DiscardRegistry;

impl CallbackRegistry<io::Error> for DiscardRegistry {
    fn add_litellm_callback(&mut self, _guardrail: Arc<LLMAsAJudgeGuardrail<io::Error>>) {}
}

#[derive(serde::Serialize)]
struct BlockedOutput {
    status: u16,
    detail: JudgeBlockDetail,
}

#[derive(serde::Serialize)]
struct Output {
    judge_call: Option<JudgeCompletionRequest>,
    inputs: Option<JudgeInputs>,
    eval_information: Option<EvalInformation>,
    blocked: Option<BlockedOutput>,
    guardrail_records: Option<Vec<JudgeGuardrailRecord>>,
}

impl crate::helpers::Helper for Op {
    type Input = ApplyInput;

    fn run(input: &Self::Input) -> HelperResult {
        let output = execute(input)?;
        serde_json::to_value(output).map_err(|error| failure("ValidationError", error.to_string()))
    }
}

fn execute(input: &ApplyInput) -> Result<Output, Box<HelperError>> {
    let entry = GuardrailEntry {
        guardrail_name: Some(input.guardrail.guardrail_name.as_str()),
        litellm_params: Some(LitellmParamsSource::Mapping(
            &input.guardrail.litellm_params,
        )),
    };
    let request: Arc<Mutex<Option<JudgeCompletionRequest>>> = Arc::new(Mutex::new(None));
    let completion = Arc::new(RecordingCompletion {
        outcome: input.judge.clone(),
        request: Arc::clone(&request),
    });
    let mut registry = DiscardRegistry;
    let guardrail =
        initialize_guardrail(&JudgeParams::default(), &entry, completion, &mut registry)
            .map_err(|error| failure("ValueError", error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", error.to_string()))?;
    let mut request_data = input.request_data.clone();
    let outcome = runtime.block_on(guardrail.apply_guardrail(
        input.inputs.clone(),
        &mut request_data,
        input.input_type,
    ));
    let judge_call = request
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let eval_information = request_data.metadata.eval_information.clone();
    let records = request_data.metadata.guardrail_records;
    collect(outcome, judge_call, eval_information, records)
}

fn collect(
    outcome: Result<JudgeInputs, JudgeError>,
    judge_call: Option<JudgeCompletionRequest>,
    eval_information: Option<EvalInformation>,
    records: Vec<JudgeGuardrailRecord>,
) -> Result<Output, Box<HelperError>> {
    let guardrail_records = (!records.is_empty()).then_some(records);
    match outcome {
        Ok(returned) => Ok(Output {
            judge_call,
            inputs: Some(returned),
            eval_information,
            blocked: None,
            guardrail_records,
        }),
        Err(error) => {
            let status = error.status_code().as_u16();
            match error {
                JudgeError::Blocked { detail } => Ok(Output {
                    judge_call,
                    inputs: None,
                    eval_information,
                    blocked: Some(BlockedOutput { status, detail }),
                    guardrail_records,
                }),
                failed @ (JudgeError::MissingGuardrailName
                | JudgeError::MissingJudgeModel
                | JudgeError::MissingCriteria
                | JudgeError::WeightSum { .. }
                | JudgeError::UnrepresentableWeight
                | JudgeError::WeightOverflow
                | JudgeError::InvalidOnFailure { .. }
                | JudgeError::InvalidEventHook { .. }
                | JudgeError::UnsupportedEventHook { .. }
                | JudgeError::InvalidParam { .. }) => {
                    Err(failure("RuntimeError", failed.to_string()))
                }
            }
        }
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
