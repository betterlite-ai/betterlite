use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_judge::Criterion;
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use serde::{Deserialize, Serialize};

use crate::content_filter::risk_scoring::GuardrailStatus;
use crate::generic_guardrail_contracts::GuardrailInputType;

use super::call::{JudgeCompletion, JudgeVerdict, run_judge};
use super::error::{JudgeBlockDetail, JudgeError};
use super::hooks::{EventHookConfig, event_type_for};
use super::prompt::text_under_review;
use super::record::{JUDGE_PROVIDER, JudgeGuardrailRecord};
use super::verdict::{EvalInformation, EvaluationInformation, merge, overall_score, verdicts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFailure {
    Block,
    Log,
}

impl OnFailure {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "block" => Some(Self::Block),
            "log" => Some(Self::Log),
            _ => None,
        }
    }
}

pub struct LLMAsAJudgeGuardrail<E: Error + Send + Sync + 'static> {
    pub guardrail_name: String,
    pub judge_model: String,
    pub criteria: Vec<Criterion>,
    pub overall_threshold: f64,
    pub on_failure: OnFailure,
    pub event_hook: EventHookConfig,
    pub default_on: bool,
    pub completion: Arc<dyn JudgeCompletion<Error = E>>,
    pub now: fn() -> SystemTime,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct JudgeInputs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_messages: Option<Vec<Message>>,
    #[serde(flatten)]
    pub rest: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct RequestData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<Message>>,
    pub metadata: RequestMetadata,
    #[serde(flatten)]
    pub rest: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct RequestMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_information: Option<EvalInformation>,
    #[serde(
        rename = "standard_logging_guardrail_information",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub guardrail_records: Vec<JudgeGuardrailRecord>,
    #[serde(flatten)]
    pub rest: BTreeMap<String, GuardrailValue>,
}

fn judge_messages(inputs: &JudgeInputs, request_data: &RequestData) -> Vec<Message> {
    match &inputs.structured_messages {
        Some(messages) if !messages.is_empty() => messages.clone(),
        Some(_) | None => request_data.messages.clone().unwrap_or_default(),
    }
}

type Evaluation = (GuardrailStatus, JudgeVerdict, Option<JudgeError>);

impl<E: Error + Send + Sync + 'static> LLMAsAJudgeGuardrail<E> {
    pub async fn apply_guardrail(
        &self,
        inputs: JudgeInputs,
        request_data: &mut RequestData,
        input_type: GuardrailInputType,
    ) -> Result<JudgeInputs, JudgeError> {
        let text = text_under_review(&inputs, input_type);
        if text.is_empty() {
            return Ok(inputs);
        }
        let started = (self.now)();
        let messages = judge_messages(&inputs, request_data);
        let (status, verdict, block) = self
            .evaluate(&messages, &text, input_type, &mut request_data.metadata)
            .await;
        request_data
            .metadata
            .guardrail_records
            .push(JudgeGuardrailRecord {
                guardrail_name: self.guardrail_name.clone(),
                guardrail_provider: JUDGE_PROVIDER.to_owned(),
                guardrail_mode: event_type_for(&self.event_hook, input_type)
                    .map_or_else(|| self.event_hook.clone(), EventHookConfig::Single),
                guardrail_response: Json::Object(verdict),
                guardrail_status: status,
                start_time: started,
                end_time: (self.now)(),
            });
        block.map_or(Ok(inputs), Err)
    }

    async fn evaluate(
        &self,
        messages: &[Message],
        text: &str,
        input_type: GuardrailInputType,
        metadata: &mut RequestMetadata,
    ) -> Evaluation {
        let verdict = match run_judge(self, messages, text, input_type).await {
            Ok(verdict) => verdict,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "llm_as_a_judge guardrail: judge call failed, failing open"
                );
                return (
                    GuardrailStatus::GuardrailFailedToRespond,
                    JudgeVerdict::new(),
                    None,
                );
            }
        };
        let Some(score) = overall_score(&verdict) else {
            tracing::warn!("llm_as_a_judge: invalid overall_score from judge, failing open");
            return (GuardrailStatus::Success, verdict, None);
        };
        self.record(metadata, score, &verdict);
        if score >= self.overall_threshold {
            return (GuardrailStatus::Success, verdict, None);
        }
        let block = (self.on_failure == OnFailure::Block)
            .then(|| self.blocked(score, &verdict, input_type));
        (GuardrailStatus::GuardrailIntervened, verdict, block)
    }

    fn record(&self, metadata: &mut RequestMetadata, score: f64, verdict: &JudgeVerdict) {
        merge(
            &mut metadata.eval_information,
            EvaluationInformation {
                eval_name: self.guardrail_name.clone(),
                overall_score: score,
                passed: score >= self.overall_threshold,
                judge_model: self.judge_model.clone(),
                threshold: self.overall_threshold,
                verdicts: verdicts(verdict),
            },
        );
    }

    fn blocked(
        &self,
        score: f64,
        verdict: &JudgeVerdict,
        input_type: GuardrailInputType,
    ) -> JudgeError {
        let rejected = match input_type {
            GuardrailInputType::Request => "request",
            GuardrailInputType::Response => "response",
        };
        JudgeError::Blocked {
            detail: JudgeBlockDetail {
                error: format!("LLM judge rejected {rejected}: score below threshold"),
                overall_score: score,
                threshold: self.overall_threshold,
                verdicts: verdicts(verdict),
            },
        }
    }
}
