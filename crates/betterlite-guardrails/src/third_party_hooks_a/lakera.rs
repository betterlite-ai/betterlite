use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::enums::{Role, default_roles};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::legacy_guardrails::Settings;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use secrecy::ExposeSecret as _;
use serde::Deserialize;

use super::error::ThirdPartyHookError;
use crate::guardrail_selection::gate::GuardrailSelection;
use crate::lakera::response::LakeraModerationResponse;
use crate::lakera::{LegacyLakeraModeration, ModerationCheck};

mod body;
pub mod error;

use body::{CheckBody, input_body, join_input_texts, messages_body};
use error::{LakeraHookError, rejection_error};

// enabled_roles come from the item named prompt_injection, not the lakera_prompt_injection callback name
const CONFIG_ITEM_NAME: &str = "prompt_injection";
const PROMPT_INJECTION_PATH: &str = "/v1/prompt_injection";

pub struct LakeraSelection<'call> {
    pub should_proceed: bool,
    pub dynamic_params: &'call (dyn GuardrailSelection + Send + Sync),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LakeraGate {
    Legacy,
    Checked(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LakeraHook {
    PreCall,
    Moderation,
}

pub struct LegacyLakeraHooks {
    pub moderation: LegacyLakeraModeration,
    pub roles: Vec<Role>,
    pub add_function_to_prompt: bool,
}

impl LegacyLakeraHooks {
    #[must_use]
    pub fn new(
        moderation: LegacyLakeraModeration,
        legacy: &Settings,
        add_function_to_prompt: bool,
    ) -> Self {
        let roles = legacy
            .by_name
            .get(CONFIG_ITEM_NAME)
            .and_then(|item| item.enabled_roles.as_ref())
            .cloned()
            .unwrap_or_else(default_roles);
        Self {
            moderation,
            roles,
            add_function_to_prompt,
        }
    }

    pub async fn hook(
        &self,
        hook: LakeraHook,
        data: &BTreeMap<String, GuardrailValue>,
        gate: LakeraGate,
        selection: &LakeraSelection<'_>,
    ) -> Result<(), ThirdPartyHookError> {
        let legacy_check = match hook {
            LakeraHook::PreCall => ModerationCheck::InParallel,
            LakeraHook::Moderation => ModerationCheck::PreCall,
        };
        let skips = match gate {
            LakeraGate::Legacy => self.moderation.moderation_check == legacy_check,
            LakeraGate::Checked(should_run) => !should_run,
        };
        if skips {
            return Ok(());
        }
        self.check(data, selection).await
    }

    pub async fn check(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
        selection: &LakeraSelection<'_>,
    ) -> Result<(), ThirdPartyHookError> {
        if !selection.should_proceed {
            return Ok(());
        }
        let dynamic = selection
            .dynamic_params
            .dynamic_request_body_params(data)
            .map_err(LakeraHookError::from)?;
        let body = match self.request_body(data, &dynamic)? {
            CheckBody::Skip => return Ok(()),
            CheckBody::Empty => String::new(),
            CheckBody::Json(value) => serde_json::to_string(&value)
                .map_err(|source| LakeraHookError::Encode { source })?,
        };
        tracing::debug!(body = %body, "Lakera AI Request Args");
        let response = self.post(&body).await?;
        self.handle_response(response).await
    }

    async fn post(&self, body: &str) -> Result<reqwest::Response, ThirdPartyHookError> {
        let url = format!("{}{PROMPT_INJECTION_PATH}", self.moderation.api_base);
        let mut authorization = HeaderValue::from_str(&format!(
            "Bearer {}",
            self.moderation.api_key.expose_secret()
        ))
        .map_err(|source| LakeraHookError::Header { source })?;
        authorization.set_sensitive(true);
        // LiteLLM posts through the guardrail's own async handler, so that client carries the timeout
        self.moderation
            .async_handler
            .post(url)
            .header(AUTHORIZATION, authorization)
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_owned())
            .send()
            .await
            .map_err(|source| ThirdPartyHookError::Lakera(LakeraHookError::Transport { source }))
    }

    async fn handle_response(
        &self,
        response: reqwest::Response,
    ) -> Result<(), ThirdPartyHookError> {
        let status = response.status();
        if status == StatusCode::OK {
            return self.checked(response).await;
        }
        // any other 2xx answer is accepted unchecked; an error status fails with its body as the text
        if status.is_success() {
            return Ok(());
        }
        let text = response
            .text()
            .await
            .map_err(|source| LakeraHookError::Transport { source })?;
        Err(ThirdPartyHookError::Failure { message: text })
    }

    async fn checked(&self, response: reqwest::Response) -> Result<(), ThirdPartyHookError> {
        let text = response
            .text()
            .await
            .map_err(|source| LakeraHookError::Transport { source })?;
        let parsed: LakeraModerationResponse = serde_json::from_str(&text)
            .map_err(|source| LakeraHookError::ResponseJson { source })?;
        if let Some(rejection) = self.moderation.check_response_flagged(&parsed) {
            return rejection_error(&rejection);
        }
        Ok(())
    }

    fn request_body(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
        dynamic: &BTreeMap<String, GuardrailValue>,
    ) -> Result<CheckBody, ThirdPartyHookError> {
        if let Some(GuardrailValue::Array(messages)) = data.get("messages") {
            return Ok(messages_body(
                messages,
                &self.roles,
                self.add_function_to_prompt,
                dynamic,
            )?);
        }
        if let Some(GuardrailValue::Text(text)) = data.get("input") {
            return Ok(CheckBody::Json(input_body(text, dynamic)));
        }
        if let Some(GuardrailValue::Array(items)) = data.get("input") {
            return Ok(CheckBody::Json(input_body(
                &join_input_texts(items)?,
                dynamic,
            )));
        }
        Ok(CheckBody::Empty)
    }
}
