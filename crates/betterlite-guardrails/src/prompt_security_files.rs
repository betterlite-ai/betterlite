use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::timeout;
use transport::upload_and_poll;
use wire::{Action, SanitizeResult};

use error::FileSanitizationError;

pub mod data_url;
mod document;
pub mod error;
mod image;
pub mod log;
pub mod messages;
mod transport;
pub mod wire;

const FILE_SANITIZATION_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_POLL_ATTEMPTS: u32 = 30;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct FileSanitizerConfig {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub request_timeout: Duration,
    pub file_sanitization_timeout: Option<Duration>,
    pub file_sanitization_fail_open: Option<bool>,
    pub block_on_file_modify: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct FileSanitizerSettings {
    pub api_key: SecretString,
    pub api_base: String,
    pub request_timeout: Duration,
    pub file_sanitization_timeout: Duration,
    pub fail_open: bool,
    pub block_on_file_modify: bool,
    pub max_poll_attempts: u32,
    pub poll_interval: Duration,
    pub max_response_bytes: usize,
}

impl TryFrom<FileSanitizerConfig> for FileSanitizerSettings {
    type Error = FileSanitizationError;

    fn try_from(config: FileSanitizerConfig) -> Result<Self, Self::Error> {
        let api_key = config
            .api_key
            .filter(|key| !key.expose_secret().is_empty())
            .ok_or(FileSanitizationError::MissingSecrets)?;
        let api_base = config
            .api_base
            .filter(|base| !base.is_empty())
            .ok_or(FileSanitizationError::MissingSecrets)?;
        Ok(Self {
            api_key,
            api_base,
            request_timeout: config.request_timeout,
            file_sanitization_timeout: config
                .file_sanitization_timeout
                .unwrap_or(FILE_SANITIZATION_TIMEOUT),
            fail_open: config.file_sanitization_fail_open.unwrap_or(true),
            block_on_file_modify: config.block_on_file_modify.unwrap_or(true),
            max_poll_attempts: MAX_POLL_ATTEMPTS,
            poll_interval: POLL_INTERVAL,
            max_response_bytes: MAX_RESPONSE_BYTES,
        })
    }
}

#[derive(Debug, Clone)]
pub struct FileSanitizer {
    pub settings: FileSanitizerSettings,
    pub client: reqwest::Client,
}

impl FileSanitizer {
    pub async fn sanitize_file_content(
        &self,
        file_data: &[u8],
        filename: &str,
        user_api_key_alias: Option<&str>,
    ) -> Result<SanitizeResult, FileSanitizationError> {
        let outcome = timeout(
            self.settings.file_sanitization_timeout,
            upload_and_poll(self, file_data, filename, user_api_key_alias),
        )
        .await;
        let cause = match outcome {
            Ok(Err(FileSanitizationError::RequestTimeout { source })) => source.to_string(),
            Ok(settled) => return settled,
            Err(elapsed) => elapsed.to_string(),
        };
        if !self.settings.fail_open {
            tracing::error!(
                filename,
                cause,
                "Prompt Security Guardrail: file sanitization timed out; failing closed"
            );
            return Err(FileSanitizationError::Timeout);
        }
        tracing::error!(
            filename,
            cause,
            "Prompt Security Guardrail: file sanitization timed out; failing open"
        );
        Ok(SanitizeResult {
            action: Action::Allow,
            content: None,
            violations: Vec::new(),
        })
    }

    pub fn raise_if_file_blocked(
        &self,
        result: &SanitizeResult,
        resource: &'static str,
    ) -> Result<(), FileSanitizationError> {
        let modify_blocks = result.action == Action::Modify && self.settings.block_on_file_modify;
        if result.action != Action::Block && !modify_blocks {
            return Ok(());
        }
        Err(FileSanitizationError::Blocked {
            resource,
            violations: result.violations.join(", "),
        })
    }
}

#[path = "prompt_security_files/tests/block_raise.rs"]
mod block_raise;
#[path = "prompt_security_files/tests/document_exception.rs"]
mod document_exception;
#[path = "prompt_security_files/tests/document_item.rs"]
mod document_item;
#[path = "prompt_security_files/tests/image_exception.rs"]
mod image_exception;
#[path = "prompt_security_files/tests/image_url.rs"]
mod image_url;
#[path = "prompt_security_files/tests/log_request.rs"]
mod log_request;
#[path = "prompt_security_files/tests/log_response.rs"]
mod log_response;
#[path = "prompt_security_files/tests/message_files.rs"]
mod message_files;
#[path = "prompt_security_files/tests/missing_jobid.rs"]
mod missing_jobid;
#[path = "prompt_security_files/tests/poll_exhausted.rs"]
mod poll_exhausted;
#[path = "prompt_security_files/tests/polling.rs"]
mod polling;
#[path = "prompt_security_files/tests/redact_headers.rs"]
mod redact_headers;
#[path = "prompt_security_files/tests/settings.rs"]
mod settings;
#[path = "prompt_security_files/tests/shared.rs"]
mod shared;
#[path = "prompt_security_files/tests/standalone_images.rs"]
mod standalone_images;
#[path = "prompt_security_files/tests/timeout_wrapper.rs"]
mod timeout_wrapper;
#[path = "prompt_security_files/tests/unexpected_status.rs"]
mod unexpected_status;
#[path = "prompt_security_files/tests/upload_poll.rs"]
mod upload_poll;
