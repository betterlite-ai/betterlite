use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::prompt_injection::PromptInjectionParams;
use betterlite_models::billing_text::{FormattedPromptData, PromptCallType, get_formatted_prompt};
use rust_decimal::Decimal;

use super::error::{PromptInjectionError, REJECTED_MESSAGE};
use super::keywords::generate_injection_keywords;
use super::pool::HeuristicsPool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookCallType {
    Acompletion,
    Completion,
    TextCompletion,
    Embeddings,
    ImageGeneration,
    Moderation,
    AudioTranscription,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreCallOutcome {
    Pass,
    RejectedAsMessage(String),
}

#[async_trait]
pub trait InjectionJudge: Send + Sync {
    async fn complete(
        &self,
        model: &str,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<Option<String>, PromptInjectionError>;
}

#[derive(Debug)]
pub struct PromptInjectionDetection {
    params: Option<PromptInjectionParams>,
    similarity_threshold: Decimal,
    pool: HeuristicsPool,
    keywords: Arc<[String]>,
    model_names: Option<Vec<String>>,
}

impl PromptInjectionDetection {
    pub const ENFORCES_REQUEST_CONTENT: bool = true;

    #[must_use]
    pub fn new(
        params: Option<PromptInjectionParams>,
        similarity_threshold: Decimal,
        pool: HeuristicsPool,
    ) -> Self {
        Self {
            params,
            similarity_threshold,
            pool,
            keywords: Arc::from(generate_injection_keywords()),
            model_names: None,
        }
    }

    pub fn update_environment(
        &mut self,
        model_names: Option<&[String]>,
    ) -> Result<(), PromptInjectionError> {
        self.model_names = model_names.map(<[String]>::to_vec);
        let Some(params) = self.params.as_ref() else {
            return Ok(());
        };
        if !params.llm_api_check {
            return Ok(());
        }
        let Some(names) = self.model_names.as_deref() else {
            return Err(PromptInjectionError::ModelListNotSet);
        };
        let known = params
            .llm_api_name
            .as_deref()
            .is_some_and(|name| names.iter().any(|listed| listed.as_str() == name));
        if !known {
            return Err(PromptInjectionError::InvalidLlmApiName);
        }
        Ok(())
    }

    pub async fn pre_call(
        &self,
        call_type: HookCallType,
        request: &FormattedPromptData<'_>,
    ) -> Result<PreCallOutcome, PromptInjectionError> {
        let prompt = match formatted_prompt(call_type, request) {
            Ok(Some(prompt)) => prompt,
            Ok(None) => return Ok(PreCallOutcome::Pass),
            Err(source) => {
                tracing::error!(
                    error = %source,
                    "the prompt injection pre-call hook could not build the prompt; passing the request"
                );
                return Ok(PreCallOutcome::Pass);
            }
        };
        let should_scan = self
            .params
            .as_ref()
            .is_none_or(|params| params.heuristics_check);
        if !should_scan {
            return Ok(PreCallOutcome::Pass);
        }
        self.scan(prompt).await
    }

    async fn scan(&self, prompt: String) -> Result<PreCallOutcome, PromptInjectionError> {
        let hit = match self
            .pool
            .check(
                Arc::clone(&self.keywords),
                prompt,
                self.similarity_threshold,
            )
            .await
        {
            Ok(hit) => hit,
            Err(PromptInjectionError::HeuristicsTimeout) => return Ok(PreCallOutcome::Pass),
            Err(source) => {
                tracing::error!(
                    error = %source,
                    "the prompt injection heuristics check failed; passing the request"
                );
                return Ok(PreCallOutcome::Pass);
            }
        };
        if !hit {
            return Ok(PreCallOutcome::Pass);
        }
        if self
            .params
            .as_ref()
            .and_then(|params| params.reject_as_response)
            == Some(true)
        {
            return Ok(PreCallOutcome::RejectedAsMessage(
                REJECTED_MESSAGE.to_owned(),
            ));
        }
        Err(PromptInjectionError::Rejected)
    }

    pub async fn moderation<J>(
        &self,
        judge: &J,
        call_type: HookCallType,
        request: &FormattedPromptData<'_>,
    ) -> Result<Option<bool>, PromptInjectionError>
    where
        J: InjectionJudge,
    {
        let Some(params) = self.params.as_ref() else {
            return Ok(None);
        };
        let Some(prompt) = formatted_prompt(call_type, request)? else {
            return Ok(None);
        };
        if prompt.is_empty() {
            return Ok(None);
        }
        if !params.llm_api_check || self.model_names.is_none() {
            return Ok(Some(false));
        }
        let Some(model) = params.llm_api_name.as_deref() else {
            return Ok(Some(false));
        };
        let system_prompt = params.llm_api_system_prompt.as_deref().unwrap_or_default();
        let Some(answer) = judge.complete(model, system_prompt, &prompt).await? else {
            return Ok(Some(false));
        };
        let fail_call_string = params.llm_api_fail_call_string.as_deref();
        if fail_call_string.is_some_and(|needle| answer.contains(needle)) {
            return Err(PromptInjectionError::Rejected);
        }
        Ok(Some(false))
    }
}

fn formatted_prompt(
    call_type: HookCallType,
    request: &FormattedPromptData<'_>,
) -> Result<Option<String>, PromptInjectionError> {
    let prompt_call_type = match call_type {
        HookCallType::Unsupported => return Ok(None),
        HookCallType::Embeddings => return Ok(Some(String::new())),
        HookCallType::Acompletion => PromptCallType::Acompletion,
        HookCallType::Completion => PromptCallType::Completion,
        HookCallType::TextCompletion => PromptCallType::TextCompletion,
        HookCallType::ImageGeneration => PromptCallType::ImageGeneration,
        HookCallType::Moderation => PromptCallType::Moderation,
        HookCallType::AudioTranscription => PromptCallType::AudioTranscription,
    };
    get_formatted_prompt(request, prompt_call_type)
        .map(Some)
        .map_err(|source| PromptInjectionError::Prompt { source })
}
