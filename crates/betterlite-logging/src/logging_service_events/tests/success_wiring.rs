#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_config::sections::service_callback::SettingsValues;
use betterlite_error::normalize::Normalizer;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::resolver::LoggerResolver;
use crate::logging_metadata::base64::Base64Truncator;
use crate::logging_service_events::sink::{ServiceEventError, ServiceEventSink};
use crate::logging_service_events::types::ServiceEvent;
use crate::logging_service_events::wiring::{LoggingInputs, LoggingWiring, SuccessInputs};
use crate::logging_streams::assembler::ChatStreamAssembler;
use crate::logging_success::ports::{StreamAssembler, SuccessClock};
use crate::payload::builder::PayloadBuilderConfig;
use crate::payload::deps::{PayloadSettings, PayloadSources, TagConfig};
use crate::registry::CallbackRegistry;
use crate::registry::error::RegistryError;
use crate::registry::phases::CallbackList;

struct FixedClock;

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").unwrap()
    }
}

struct NullSink;

impl ServiceEventSink for NullSink {
    fn emit(&self, _event: &ServiceEvent) -> Result<(), ServiceEventError> {
        Ok(())
    }
}

struct NamedLogger {
    name: String,
}

impl CallbackLogger for NamedLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }
}

struct FixedResolver {
    logger: Arc<dyn CallbackLogger>,
}

impl LoggerResolver for FixedResolver {
    fn resolve(
        &self,
        name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        if name == self.logger.callback_name() {
            return Ok(Some(Arc::clone(&self.logger)));
        }
        Ok(None)
    }
}

fn wiring_settings() -> SettingsValues {
    SettingsValues {
        service_callbacks: vec!["datadog".to_owned()],
        log_level: "info".to_owned(),
        json_logs: false,
        max_queue_size: 8,
        concurrency: 2,
        event_timeout: Duration::from_secs(5),
    }
}

fn payload(privacy: Settings) -> PayloadBuilderConfig {
    PayloadBuilderConfig {
        sources: Arc::new(PayloadSources::default()),
        settings: PayloadSettings {
            tag_config: TagConfig::default(),
            request_correlation_in_logs: false,
            log_client_error_tracebacks: false,
        },
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
        normalizer: Arc::new(Normalizer::new().unwrap()),
        truncator: Base64Truncator::new(4_096).unwrap(),
        privacy,
    }
}

#[tokio::test]
async fn wiring_builds_one_success_handler_over_its_parts() {
    let logger: Arc<dyn CallbackLogger> = Arc::new(NamedLogger {
        name: "datadog".to_owned(),
    });
    let registry = CallbackRegistry::new(16).with_resolver(Arc::new(FixedResolver { logger }));
    let clock: Arc<dyn SuccessClock> = Arc::new(FixedClock);
    let stream_assembler: Arc<dyn StreamAssembler> = Arc::new(ChatStreamAssembler);
    let privacy = Settings {
        turn_off_message_logging: true,
        redact_user_api_key_info: true,
    };
    let known_callback: Arc<dyn Fn(&str) -> bool + Send + Sync> =
        Arc::new(|name: &str| name == "langfuse");
    let settings = wiring_settings();
    let wiring = LoggingWiring::start(LoggingInputs {
        settings: &settings,
        clock: Arc::clone(&clock),
        sink: Arc::new(NullSink),
        registry,
        stream_assembler: Arc::clone(&stream_assembler),
        success: SuccessInputs {
            payload: payload(privacy),
            excluded_fields: vec!["messages".to_owned()],
            known_callback: Arc::clone(&known_callback),
        },
    })
    .expect("the wiring starts");
    let success = &wiring.success;
    assert!(Arc::ptr_eq(&success.stream_assembler, &stream_assembler));
    assert!(Arc::ptr_eq(&success.clock, &clock));
    let service = success.registry.list_identities(CallbackList::Service);
    assert_eq!(service.len(), 1);
    assert_eq!(service[0].class_name, "datadog");
    assert_eq!(success.excluded_fields, ["messages".to_owned()]);
    assert_eq!(success.privacy, privacy);
    assert!(success.known_callback("langfuse"));
    assert!(!success.known_callback("custom_callback"));
    wiring.pipeline.stop().await;
}
