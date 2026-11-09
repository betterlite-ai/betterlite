use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redact::secret::Redactor;
use thiserror::Error;
use tokio_util::task::TaskTracker;

use super::ports::{
    AlertLevel, AlertType, AlertingClient, BudgetAlertKind, CallInfo, Clock, EmailAlerting,
    FailureLogging, FailureService, RequestStatus, SentryCapture, ServiceLogging, SlackAlert,
    SlackAlerting, UsageCache,
};
use crate::guardrails::callback_caps::errors::ProxyHookError;

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum AlertError {
    #[error("Missing SENTRY_DSN from environment")]
    MissingSentryDsn,
}

pub struct AlertRequestData {
    pub alerting_metadata: JsonMap,
    pub logging: Option<Arc<dyn FailureLogging>>,
}

// The lookup is awaited inside the spawned alert so a port with LiteLLM's retry sleeps never blocks a request
async fn alert_context(request_data: Option<AlertRequestData>) -> (JsonMap, Option<String>) {
    let Some(request) = request_data else {
        return (JsonMap::new(), None);
    };
    let langfuse = match request.logging {
        Some(logging) => logging.langfuse_trace_url().await,
        None => None,
    };
    (request.alerting_metadata, langfuse)
}

#[must_use]
pub fn alerting_metadata_from(request_data: &JsonMap) -> JsonMap {
    request_data
        .get("metadata")
        .and_then(JsonValue::as_object)
        .and_then(|metadata| metadata.get("alerting_metadata"))
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default()
}

pub struct ProxyAlerts {
    pub alerting: Option<Vec<AlertingClient>>,
    pub alert_types: Vec<AlertType>,
    pub alerting_threshold: Duration,
    pub proxy_base_url: Option<String>,
    pub usage_cache: Arc<dyn UsageCache>,
    pub slack: Arc<dyn SlackAlerting>,
    pub email: Option<Arc<dyn EmailAlerting>>,
    pub service_logging: Arc<dyn ServiceLogging>,
    pub sentry: Option<Arc<dyn SentryCapture>>,
    pub redactor: Option<Arc<Redactor>>,
    pub clock: Arc<dyn Clock>,
    pub tasks: TaskTracker,
}

impl ProxyAlerts {
    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        self.redactor
            .as_ref()
            .map_or_else(|| text.to_owned(), |redactor| redactor.redact_string(text))
    }

    pub async fn update_request_status(&self, call_id: &str, status: RequestStatus) {
        if self.alerting.is_none() {
            return;
        }
        let ttl = self
            .alerting_threshold
            .saturating_add(Duration::from_secs(100));
        self.usage_cache
            .set_local(&format!("request_status:{call_id}"), status.as_str(), ttl)
            .await;
    }

    pub async fn failed_tracking_alert(&self, error_message: &str, failing_model: &str) {
        if self.alerting.is_none() {
            return;
        }
        self.slack
            .failed_tracking_alert(error_message, failing_model)
            .await;
    }

    pub async fn budget_alerts(&self, kind: BudgetAlertKind, user_info: &CallInfo) {
        let soft_with_emails = kind == BudgetAlertKind::SoftBudget
            && user_info
                .alert_emails
                .as_ref()
                .is_some_and(|emails| !emails.is_empty());
        if self.alerting.is_none() && !soft_with_emails {
            return;
        }
        let slack_clients = self.alerting.as_ref().is_some_and(|clients| {
            clients.iter().any(|client| {
                matches!(
                    client,
                    AlertingClient::Slack | AlertingClient::MsTeams | AlertingClient::Webhook
                )
            })
        });
        if slack_clients {
            self.slack.budget_alerts(kind, user_info).await;
        }
        let email_client = self
            .alerting
            .as_ref()
            .is_some_and(|clients| clients.contains(&AlertingClient::Email));
        if (email_client || soft_with_emails)
            && let Some(email) = &self.email
        {
            email.budget_alerts(kind, user_info).await;
        }
    }

    pub async fn alerting_handler(
        &self,
        message: &str,
        level: AlertLevel,
        alert_type: AlertType,
        request_data: Option<AlertRequestData>,
    ) -> Result<(), AlertError> {
        let Some(clients) = &self.alerting else {
            return Ok(());
        };
        let timestamp = self.clock.now().format("%H:%M:%S").to_string();
        let (alerting_metadata, langfuse) = alert_context(request_data).await;
        let formatted = self.format_alert(message, level, &timestamp, langfuse.as_deref());
        if clients
            .iter()
            .any(|client| matches!(client, AlertingClient::Slack | AlertingClient::MsTeams))
        {
            self.slack
                .send_alert(SlackAlert {
                    message,
                    level,
                    alert_type,
                    user_info: None,
                    alerting_metadata: &alerting_metadata,
                    langfuse_trace: langfuse.as_deref(),
                })
                .await;
        }
        for _client in clients
            .iter()
            .filter(|client| **client == AlertingClient::Sentry)
        {
            let Some(sentry) = &self.sentry else {
                return Err(AlertError::MissingSentryDsn);
            };
            sentry.capture_message(&formatted);
        }
        Ok(())
    }

    fn format_alert(
        &self,
        message: &str,
        level: AlertLevel,
        timestamp: &str,
        langfuse: Option<&str>,
    ) -> String {
        let mut formatted =
            format!("Level: `{level}`\nTimestamp: `{timestamp}`\n\nMessage: {message}");
        let proxy_line = self
            .proxy_base_url
            .as_ref()
            .map(|base| format!("\n\nProxy URL: `{base}`"));
        if let Some(line) = proxy_line {
            formatted.push_str(&line);
        }
        let langfuse_line = langfuse.map(|url| format!("\n\n🪢 Langfuse Trace: {url}"));
        if let Some(line) = langfuse_line {
            formatted.push_str(&line);
        }
        formatted
    }

    pub async fn failure_handler(
        self: &Arc<Self>,
        error: &ProxyHookError,
        duration: Duration,
        call_type: &str,
        traceback: &str,
    ) {
        if !self.alert_types.contains(&AlertType::DbExceptions) {
            return;
        }
        let redacted = self.failure_message(error, traceback);
        let alert_message = format!("DB read/write call failed: {redacted}");
        let alerts = Arc::clone(self);
        let _task = self.tasks.spawn(async move {
            let request = AlertRequestData {
                alerting_metadata: JsonMap::new(),
                logging: None,
            };
            let sent = alerts
                .alerting_handler(
                    &alert_message,
                    AlertLevel::High,
                    AlertType::DbExceptions,
                    Some(request),
                )
                .await;
            if let Err(sent_error) = sent {
                tracing::error!("the db failure alert could not be sent: {sent_error}");
            }
        });
        self.service_logging
            .async_service_failure_hook(FailureService::Db, duration, &redacted, call_type)
            .await;
        if let Some(sentry) = &self.sentry {
            sentry.capture_exception(error);
        }
    }

    fn failure_message(&self, error: &ProxyHookError, traceback: &str) -> String {
        let mut message = match error {
            ProxyHookError::Http { detail, .. } => detail.clone(),
            other @ (ProxyHookError::Proxy(_)
            | ProxyHookError::GuardrailRaised { .. }
            | ProxyHookError::SensitiveDataRoute(_)
            | ProxyHookError::ModifyResponse(_)
            | ProxyHookError::RejectedRequest { .. }
            | ProxyHookError::Other(_)) => other.to_string(),
        };
        message.extend(traceback.chars().take(1000));
        self.redact(&message)
    }

    pub fn start_hanging_request_check(self: &Arc<Self>, data: Option<JsonMap>) {
        if !self.slack.alerting_configured() {
            return;
        }
        let slack = Arc::clone(&self.slack);
        let _task = self.tasks.spawn(async move {
            slack.response_taking_too_long(data).await;
        });
    }
}
