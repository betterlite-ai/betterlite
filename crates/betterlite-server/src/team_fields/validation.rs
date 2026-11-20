use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use betterlite_config::sections::team_fields::Settings as TeamFieldsSettings;
use betterlite_config::sections::team_fields::ValidationTimeout;
use betterlite_db::budget_permission_rows::JsonData;

use crate::team_request_contracts::metadata::TeamMetadataFieldSchema;

use super::metadata::{
    DEFAULT_REJECTED_MESSAGE, MetadataRequest, TeamMetadataValidationPayload,
    TeamMetadataValidationResult, build_payload,
};
use super::python_validator::ValidatorFailure;

pub const NOT_ASYNC_MESSAGE: &str = "custom_team_metadata_validate must be an async function";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TeamMetadataError {
    #[error("{}", NOT_ASYNC_MESSAGE)]
    NotAsync,
    #[error("{message}")]
    Unavailable { message: String },
    #[error("{message}")]
    Rejected { message: String },
}

impl TeamMetadataError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::NotAsync => 500,
            Self::Unavailable { .. } => 503,
            Self::Rejected { .. } => 400,
        }
    }

    #[must_use]
    pub const fn message(&self) -> &str {
        match self {
            Self::NotAsync => NOT_ASYNC_MESSAGE,
            Self::Unavailable { message } | Self::Rejected { message } => message.as_str(),
        }
    }
}

#[async_trait]
pub trait TeamMetadataValidator: Send + Sync {
    fn is_async(&self) -> bool;

    async fn invoke(
        &self,
        payload: &TeamMetadataValidationPayload,
        timeout: ValidationTimeout,
    ) -> Result<JsonData, ValidatorFailure>;
}

pub async fn run_team_metadata_validation(
    validator: &dyn TeamMetadataValidator,
    payload: &TeamMetadataValidationPayload,
    timeout: ValidationTimeout,
    unavailable_message: &str,
) -> Result<(), TeamMetadataError> {
    if !validator.is_async() {
        return Err(TeamMetadataError::NotAsync);
    }
    let result = validator.invoke(payload, timeout).await;
    finish_validation(result, unavailable_message)
}

pub fn finish_validation(
    result: Result<JsonData, ValidatorFailure>,
    unavailable_message: &str,
) -> Result<(), TeamMetadataError> {
    let value = match result {
        Ok(value) => value,
        Err(failure) => {
            tracing::warn!(error = %failure, "team metadata validation failed");
            return Err(TeamMetadataError::Unavailable {
                message: unavailable_message.to_owned(),
            });
        }
    };
    let parsed = match TeamMetadataValidationResult::from_data(&value) {
        Ok(parsed) => parsed,
        Err(_) => {
            tracing::warn!(
                "the metadata validator returned a result that is not a validation result"
            );
            return Err(TeamMetadataError::Unavailable {
                message: unavailable_message.to_owned(),
            });
        }
    };
    if parsed.valid {
        return Ok(());
    }
    Err(TeamMetadataError::Rejected {
        message: rejection_message(parsed.error_message.as_deref()),
    })
}

// LiteLLM's `or` reads an empty message as missing, while a message of spaces is used as written.
fn rejection_message(error_message: Option<&str>) -> String {
    match error_message {
        Some(message) if !message.is_empty() => message.to_owned(),
        Some(_) | None => DEFAULT_REJECTED_MESSAGE.to_owned(),
    }
}

#[derive(Default)]
pub struct TeamMetadataValidatorRegistry {
    validator: RwLock<Option<Arc<dyn TeamMetadataValidator>>>,
}

impl TeamMetadataValidatorRegistry {
    pub fn set(&self, validator: Option<Arc<dyn TeamMetadataValidator>>) {
        match self.validator.write() {
            Ok(mut current) => *current = validator,
            Err(poisoned) => *poisoned.into_inner() = validator,
        }
    }

    #[must_use]
    pub fn get(&self) -> Option<Arc<dyn TeamMetadataValidator>> {
        match self.validator.read() {
            Ok(current) => (*current).clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[derive(Default)]
pub struct TeamMetadataSchemaRegistry {
    fields: RwLock<Vec<TeamMetadataFieldSchema>>,
}

impl TeamMetadataSchemaRegistry {
    pub fn set(&self, fields: Vec<TeamMetadataFieldSchema>) {
        match self.fields.write() {
            Ok(mut current) => *current = fields,
            Err(poisoned) => *poisoned.into_inner() = fields,
        }
    }

    #[must_use]
    pub fn get(&self) -> Vec<TeamMetadataFieldSchema> {
        match self.fields.read() {
            Ok(current) => (*current).clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

// ast-grep-ignore: no-long-names - LiteLLM names this validate_team_metadata_if_configured
pub async fn validate_team_metadata_if_configured(
    request: MetadataRequest<'_>,
    validators: &TeamMetadataValidatorRegistry,
    settings: &TeamFieldsSettings,
) -> Result<(), TeamMetadataError> {
    let Some(validator) = validators.get() else {
        return Ok(());
    };
    let payload = build_payload(request);
    run_team_metadata_validation(
        validator.as_ref(),
        &payload,
        settings.timeout,
        &settings.unavailable_message,
    )
    .await
}
