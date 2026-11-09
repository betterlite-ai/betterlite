use std::sync::Arc;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::alerts::{AlertRequestData, ProxyAlerts, alerting_metadata_from};
use super::lift::failure_fields_to_lift;
use super::ports::{
    AlertLevel, AlertType, DeploymentCatalog, FailureLogging, FailureTokenCounter, LoggingFactory,
    RequestStatus, RouteCallTypes,
};
use super::proxy_only::{handle_proxy_only_error, is_proxy_only_error};
use crate::guardrails::callback_caps::errors::{HttpException, ProxyHookError, is_client_error};
use crate::guardrails::callback_caps::traits::{FailureReply, ProxyCallback};

pub struct FailureHook {
    pub alerts: Arc<ProxyAlerts>,
    pub logging_factory: Arc<dyn LoggingFactory>,
    pub token_counter: Arc<dyn FailureTokenCounter>,
    pub deployments: Arc<dyn DeploymentCatalog>,
    pub route_call_types: Arc<dyn RouteCallTypes>,
    pub new_call_id: Arc<dyn Fn() -> String + Send + Sync>,
    pub log_client_error_tracebacks: bool,
}

pub struct FailureKey {
    pub request_route: Option<String>,
    pub team_id: Option<String>,
    pub logged_key_metadata: JsonMap,
}

pub struct FailureHookInput<'request> {
    pub request_data: &'request mut JsonMap,
    pub error: &'request ProxyHookError,
    pub auth: &'request UserAPIKeyAuth,
    pub key: FailureKey,
    pub error_type: Option<ProxyErrorTypes>,
    pub route: Option<&'request str>,
    pub traceback: Option<&'request str>,
    pub logging: Option<Arc<dyn FailureLogging>>,
    pub callbacks: &'request [Arc<dyn ProxyCallback>],
}

impl FailureHook {
    pub async fn post_call_failure_hook(
        &self,
        mut input: FailureHookInput<'_>,
    ) -> Option<HttpException> {
        if let Some(logging) = &input.logging
            && logging.has_baseline_context()
        {
            logging
                .invalidate_baseline_estimate("failed_request", true)
                .await;
        }
        let call_id = input
            .request_data
            .get("litellm_call_id")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        self.alerts
            .update_request_status(&call_id, RequestStatus::Fail)
            .await;
        self.alert_llm_exception(&input);
        input.request_data.remove("standard_logging_object");
        if is_proxy_only_error(
            input.error,
            input.error_type,
            input.key.request_route.as_deref(),
        ) {
            handle_proxy_only_error(self, &mut input).await;
        }
        self.lift_failure_fields(&mut input).await;
        input.request_data.remove("litellm_logging_obj");
        let traceback = input.traceback.map(|text| self.alerts.redact(text));
        self.dispatch_callbacks(&mut input, traceback.as_deref())
            .await
    }

    fn alert_llm_exception(&self, input: &FailureHookInput<'_>) {
        if !self.alerts.alert_types.contains(&AlertType::LlmExceptions)
            || is_client_error(input.error)
        {
            return;
        }
        let raw = format!("LLM API call failed: `{}`", input.error);
        let message = self.alerts.redact(&raw);
        let alerting_metadata = alerting_metadata_from(input.request_data);
        let logging = input.logging.clone();
        let alerts = Arc::clone(&self.alerts);
        let _task = self.alerts.tasks.spawn(async move {
            let request = AlertRequestData {
                alerting_metadata,
                logging,
            };
            let sent = alerts
                .alerting_handler(
                    &message,
                    AlertLevel::High,
                    AlertType::LlmExceptions,
                    Some(request),
                )
                .await;
            if let Err(error) = sent {
                tracing::error!("the llm exception alert could not be sent: {error}");
            }
        });
    }

    async fn lift_failure_fields(&self, input: &mut FailureHookInput<'_>) {
        let Some(logging) = &input.logging else {
            return;
        };
        let details = logging.details();
        let lifted =
            failure_fields_to_lift(&details, input.request_data, self.token_counter.as_ref()).await;
        match lifted {
            Ok(fields) => fields.apply(input.request_data),
            Err(error) => tracing::error!("the failure fields were not lifted: {error}"),
        }
    }

    async fn dispatch_callbacks(
        &self,
        input: &mut FailureHookInput<'_>,
        traceback: Option<&str>,
    ) -> Option<HttpException> {
        let callbacks = input.callbacks;
        let mut transformed = None;
        for callback in callbacks {
            let reply = callback
                .post_call_failure(input.request_data, input.error, input.auth, traceback)
                .await;
            match reply {
                Ok(FailureReply::Original) => {}
                Ok(FailureReply::Replace(exception)) => {
                    first_http_error(&mut transformed, exception);
                }
                Err(ProxyHookError::Http { status, detail }) => {
                    first_http_error(&mut transformed, HttpException { status, detail });
                }
                Err(error) => tracing::error!(
                    "[Non-Blocking] Error in async_post_call_failure_hook callback: {error}"
                ),
            }
        }
        transformed
    }
}

fn first_http_error(transformed: &mut Option<HttpException>, exception: HttpException) {
    if transformed.is_none() {
        *transformed = Some(exception);
    }
}
