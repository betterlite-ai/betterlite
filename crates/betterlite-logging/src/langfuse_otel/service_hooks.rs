use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use opentelemetry_sdk::trace::Span as SdkSpan;

use super::LangfuseOtelLogger;
use crate::otel::model::payloads::service::ServiceLoggerPayload;

// Langfuse only receives spans for LLM calls, so internal proxy services (auth, postgres, proxy_pre_call) never reach it.
#[async_trait::async_trait]
pub trait ServiceHooks {
    fn request_started_span(
        &self,
        start_time: DateTime<Utc>,
        headers: &BTreeMap<String, String>,
    ) -> Option<SdkSpan>;

    async fn async_service_success_hook(&self, payload: &ServiceLoggerPayload);

    async fn async_service_failure_hook(&self, payload: &ServiceLoggerPayload);
}

#[async_trait::async_trait]
impl ServiceHooks for LangfuseOtelLogger {
    fn request_started_span(
        &self,
        _start_time: DateTime<Utc>,
        _headers: &BTreeMap<String, String>,
    ) -> Option<SdkSpan> {
        None
    }

    async fn async_service_success_hook(&self, _payload: &ServiceLoggerPayload) {}

    async fn async_service_failure_hook(&self, _payload: &ServiceLoggerPayload) {}
}
