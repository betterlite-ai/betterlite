use std::collections::BTreeMap;
use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::judge_primitives::judge_verdict_parsing::parse_json_verdict;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;
use serde::Serialize;

use crate::generic_guardrail_contracts::GuardrailInputType;

use self::error::JudgeCallError;
use super::apply::LLMAsAJudgeGuardrail;
use super::hooks::{INTERNAL_CALL_ORIGIN, JUDGE_CALL_ORIGIN};
use super::prompt::{build_judge_prompt, system_prompt};

pub mod error;

pub type JudgeVerdict = IndexMap<String, Json>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JudgeMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JudgeResponseFormat {
    JsonObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JudgeCompletionRequest {
    pub model: String,
    pub messages: Vec<JudgeMessage>,
    pub response_format: JudgeResponseFormat,
    pub temperature: u8,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeReply {
    pub content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeCompletionResponse {
    pub choices: Vec<JudgeReply>,
}

#[async_trait]
pub trait JudgeCompletion: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn complete(
        &self,
        request: &JudgeCompletionRequest,
    ) -> Result<JudgeCompletionResponse, Self::Error>;
}

pub async fn run_judge<E>(
    guardrail: &LLMAsAJudgeGuardrail<E>,
    messages: &[Message],
    text_under_review: &str,
    input_type: GuardrailInputType,
) -> Result<JudgeVerdict, JudgeCallError<E>>
where
    E: Error + Send + Sync + 'static,
{
    let request = JudgeCompletionRequest {
        model: guardrail.judge_model.clone(),
        messages: vec![
            JudgeMessage {
                role: "system".to_owned(),
                content: system_prompt(input_type).to_owned(),
            },
            JudgeMessage {
                role: "user".to_owned(),
                content: build_judge_prompt(
                    &guardrail.criteria,
                    messages,
                    text_under_review,
                    input_type,
                ),
            },
        ],
        response_format: JudgeResponseFormat::JsonObject,
        temperature: 0,
        metadata: BTreeMap::from([(
            INTERNAL_CALL_ORIGIN.to_owned(),
            JUDGE_CALL_ORIGIN.to_owned(),
        )]),
    };
    let response = guardrail
        .completion
        .complete(&request)
        .await
        .map_err(|source| JudgeCallError::Completion { source })?;
    let reply = response.choices.first().ok_or(JudgeCallError::NoChoices)?;
    let content = reply
        .content
        .as_deref()
        .filter(|text| !text.is_empty())
        .unwrap_or("{}");
    parse_json_verdict(content).map_err(|source| JudgeCallError::Verdict { source })
}
