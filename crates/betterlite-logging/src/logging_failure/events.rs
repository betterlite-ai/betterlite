use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use indexmap::IndexMap;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::event::LoggerEvent;
// ast-grep-ignore: no-long-names
use crate::exporter_options::redaction::redact_standard_logging_payload;
use crate::exporter_options::redaction::{
    ModelCallDetails, Redaction, RedactionPolicy, ResponseStamp,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_failure::FailureError;
use crate::logging_failure::request_redaction::RequestRedaction;

pub struct FailureDelivery<'delivery> {
    pub payload: &'delivery Arc<StandardLoggingPayload>,
    pub dynamic_params: Arc<StandardCallbackDynamicParams>,
    pub litellm_metadata: Arc<IndexMap<String, Json>>,
    pub stream: bool,
    pub excluded_fields: &'delivery [String],
    pub stamp: ResponseStamp,
    pub request_redaction: RequestRedaction,
}

impl FailureDelivery<'_> {
    pub fn event_for(&self, logger: &dyn CallbackLogger) -> Result<LoggerEvent, FailureError> {
        Ok(LoggerEvent {
            payload: self.redacted_payload(logger)?,
            dynamic_params: Arc::clone(&self.dynamic_params),
            litellm_metadata: Arc::clone(&self.litellm_metadata),
            stream: self.stream,
            guardrail_snapshot: None,
        })
    }

    pub fn deliver_sync(&self, logger: &dyn CallbackLogger) {
        match self.event_for(logger) {
            Ok(event) => {
                if let Err(error) = logger.log_failure_event(&event) {
                    tracing::warn!(%error, "a sync failure callback errored");
                }
            }
            Err(error) => tracing::warn!(
                %error,
                callback = logger.callback_name(),
                "the failure payload could not be redacted for a callback"
            ),
        }
    }

    fn redacted_payload(
        &self,
        logger: &dyn CallbackLogger,
    ) -> Result<Arc<StandardLoggingPayload>, FailureError> {
        let turn_off_message_logging =
            self.request_redaction == RequestRedaction::Redact || logger.turn_off_message_logging();
        if !turn_off_message_logging && self.excluded_fields.is_empty() {
            return Ok(Arc::clone(self.payload));
        }
        let payload_map = payload_to_map(self.payload.as_ref())?;
        let policy = RedactionPolicy {
            turn_off_message_logging,
            excluded_fields: removable_fields(&payload_map, self.excluded_fields),
            callback_redacts_itself: logger.redacts_messages_itself(),
        };
        let details = ModelCallDetails {
            standard_logging_object: Some(payload_map),
            proxy_server_request: None,
        };
        // ast-grep-ignore: no-long-names
        let redacted = redact_standard_logging_payload(&details, &policy, &self.stamp);
        let Redaction::Redacted {
            standard_logging_object,
            ..
        } = redacted
        else {
            return Ok(Arc::clone(self.payload));
        };
        Ok(Arc::new(map_to_payload(&standard_logging_object)?))
    }
}

fn removable_fields(payload: &IndexMap<String, Json>, excluded: &[String]) -> Vec<String> {
    let mut removable = Vec::new();
    for field in excluded {
        let mut probe = payload.clone();
        probe.shift_remove(field);
        if map_to_payload(&probe).is_ok() {
            removable.push(field.clone());
        } else {
            tracing::warn!(
                field,
                "an excluded payload field is required, so the logger still gets it"
            );
        }
    }
    removable
}

pub fn payload_to_map(
    payload: &StandardLoggingPayload,
) -> Result<IndexMap<String, Json>, FailureError> {
    let encoded =
        serde_json::to_vec(payload).map_err(|source| FailureError::PayloadEncode { source })?;
    serde_json::from_slice(&encoded).map_err(|source| FailureError::PayloadDecode { source })
}

pub fn map_to_payload(
    object: &IndexMap<String, Json>,
) -> Result<StandardLoggingPayload, FailureError> {
    let encoded =
        serde_json::to_vec(object).map_err(|source| FailureError::PayloadEncode { source })?;
    serde_json::from_slice(&encoded).map_err(|source| FailureError::PayloadDecode { source })
}
