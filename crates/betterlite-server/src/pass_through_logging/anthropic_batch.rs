use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::user_roles::UserRole;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::batches::records::BatchObject;
use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod creation;
mod error;
mod records;
mod registration;
mod tests;

const UNIFIED_ID_PREFIX: &str = "litellm_proxy;model_id:";
const UNIFIED_ID_BATCH_MARKER: &str = ";llm_batch_id:";
const ANTHROPIC_PREFIX: &str = "anthropic/";
const UNKNOWN_MODEL: &str = "unknown";
const FAILED_MODEL: &str = "anthropic_batch";
const BATCH_OBJECT: &str = "batch";
const FILE_PURPOSE: &str = "batch";
const CHOICE_FINISH_REASON: &str = "stop";
const ASSISTANT_ROLE: &str = "assistant";
const DEFAULT_USER_ID: &str = "default-user";
const SUCCESS_STATUS: u16 = 200;

pub trait RouterModelIds: Send + Sync {
    fn model_ids(&self, model_name: &str) -> Vec<String>;
}

pub trait ManagedBatchRegistry: Send + Sync {
    fn store_unified_object_id(
        &self,
        request: BatchRegistration,
    ) -> BoxFuture<'static, Result<(), RegistryError>>;
}

pub trait ResponseIds: Send + Sync {
    fn new_response_id(&self) -> String;
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct RegistryError {
    pub message: String,
}

pub struct AnthropicBatchLogging {
    router: Option<Arc<dyn RouterModelIds>>,
    registry: Option<Arc<dyn ManagedBatchRegistry>>,
    ids: Arc<dyn ResponseIds>,
}

#[derive(Clone, Copy)]
pub struct BatchCreationInput<'input> {
    pub status_code: u16,
    pub body: &'input [u8],
    pub url_route: &'input str,
    pub start_time: DateTime<Utc>,
    pub request_body: Option<&'input Json>,
    pub request_metadata: &'input JsonObject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchCreationLogged {
    pub result: BatchLoggingResponse,
    pub kwargs: BatchLoggingKwargs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BatchLoggingResponse {
    pub id: String,
    pub model: String,
    pub object: String,
    pub created: i64,
    pub choices: Vec<BatchLoggingChoice>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BatchLoggingChoice {
    pub finish_reason: String,
    pub index: u32,
    pub message: BatchLoggingMessage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BatchLoggingMessage {
    pub role: String,
    pub content: String,
    pub tool_calls: Option<Json>,
    pub function_call: Option<Json>,
    pub provider_specific_fields: IndexMap<String, Json>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchLoggingKwargs {
    pub response_cost: Decimal,
    pub model: String,
    pub batch_id: Option<String>,
    pub unified_object_id: Option<String>,
    pub batch_job_state: BatchJobState,
    pub request_metadata: JsonObject,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchJobState {
    InProgress,
    Failed,
}

impl BatchJobState {
    const fn word(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BatchRegistration {
    pub unified_object_id: String,
    pub batch_object: BatchObject,
    pub model_object_id: String,
    pub file_purpose: String,
    pub registrant: BatchRegistrant,
    pub request_tags: Option<Vec<String>>,
    pub persist_attribution: bool,
    pub create_if_missing: bool,
}

#[derive(Clone, Debug)]
pub struct BatchRegistrant {
    pub user_id: Option<String>,
    pub api_key: Option<SecretString>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub user_role: UserRole,
    pub user_email: Option<String>,
    pub max_budget: Option<Decimal>,
    pub spend: Decimal,
    pub models: Vec<String>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub budget_duration: Option<String>,
    pub budget_reset_at: Option<DateTime<Utc>>,
    pub max_parallel_requests: Option<i64>,
    pub allowed_model_region: Option<String>,
    pub metadata: JsonObject,
    pub key_alias: Option<String>,
    pub permissions: JsonObject,
    pub model_max_budget: JsonObject,
    pub model_spend: JsonObject,
}

impl AnthropicBatchLogging {
    #[must_use]
    pub const fn new(
        router: Option<Arc<dyn RouterModelIds>>,
        registry: Option<Arc<dyn ManagedBatchRegistry>>,
        ids: Arc<dyn ResponseIds>,
    ) -> Self {
        Self {
            router,
            registry,
            ids,
        }
    }

    pub fn batch_creation_handler(
        &self,
        input: BatchCreationInput<'_>,
        call_log: &mut LoggingState,
    ) -> BatchCreationLogged {
        match creation::handle(self, &input, call_log) {
            Ok(logged) => logged,
            Err(error) => {
                tracing::error!("Error in batch_creation_handler: {error}");
                let response_id = self.ids.new_response_id();
                records::creation_failed(
                    &input,
                    response_id,
                    records::CreationFailure::Error(error.to_string()),
                )
            }
        }
    }

    pub fn store_batch_managed_object(
        &self,
        unified_object_id: &str,
        batch_object: BatchObject,
        model_object_id: &str,
        request_metadata: &JsonObject,
    ) {
        if let Err(error) = registration::store(
            self,
            unified_object_id,
            batch_object,
            model_object_id,
            request_metadata,
        ) {
            tracing::error!("Error storing Anthropic batch managed object: {error}");
        }
    }

    #[must_use]
    // ast-grep-ignore: no-long-names
    pub fn get_actual_model_id_from_router(&self, model_name: &str) -> String {
        let Some(router) = self.router.as_ref() else {
            tracing::warn!("Router not available, using model name: {model_name}");
            return model_name.to_owned();
        };
        let Some(model_id) = router.model_ids(model_name).into_iter().next() else {
            tracing::warn!("Model not found in router, using model name: {model_name}");
            return model_name.to_owned();
        };
        tracing::info!("Found model ID in router: {model_id}");
        model_id
    }
}

#[must_use]
pub fn unified_object_id(model_id: &str, batch_id: &str) -> String {
    let unified_id_string =
        format!("{UNIFIED_ID_PREFIX}{model_id}{UNIFIED_ID_BATCH_MARKER}{batch_id}");
    URL_SAFE_NO_PAD.encode(unified_id_string)
}
