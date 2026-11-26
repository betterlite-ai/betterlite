use async_trait::async_trait;
use axum::http::StatusCode;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_logging::exporter_options::health::{HealthState, IntegrationHealthStatus};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::auth::auth_identity::AuthContext;
use crate::health_access_and_connections::visibility::{CallerError, HealthCaller, is_proxy_admin};
use crate::health_readiness::callback_listing::CallbackEntry;
use crate::health_services::alert_tests::{self, AlertSetup};
use crate::health_services::callback_id::get_callback_identifier;
use crate::health_services::mock_requests::{MOCK_REQUEST, MockRequests};
use crate::health_services::service_name::ServiceName;
use crate::health_services::service_routes::{HealthBody, ServiceBody, SuccessBody};

// every injected failure becomes the endpoint's 500 "Authentication Error, <text>"
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ServiceTestError {
    pub message: String,
}

// PointFiveLogger's missing-key ValueError: its text is the 200 answer, not a server error
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct PointFiveSetupError {
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{detail}")]
    Rejection { status: StatusCode, detail: String },
    #[error(transparent)]
    Caller(#[from] CallerError),
    #[error("the caller's key could not be read: {0}")]
    KeyFields(#[source] serde_json::Error),
    #[error(transparent)]
    Test(#[from] ServiceTestError),
    #[error("langfuse auth_check failed: {reason}")]
    LangfuseAuthCheck { reason: String },
}

// user_api_key_dict as the alert tests read it, plus B4446's admin role check
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Caller {
    pub token: String,
    pub user_id: Option<String>,
    pub key_alias: Option<String>,
    pub team_id: Option<String>,
    pub proxy_admin: bool,
}

impl Caller {
    pub fn from_context(context: &AuthContext) -> Result<Self, ServiceError> {
        let health = HealthCaller::from_context(context)?;
        let proxy_admin = is_proxy_admin(&health);
        let fields = key_alert_fields(context.principal.source_key.0.as_ref())?;
        let token = match fields.token {
            Some(secret) => secret.expose_secret().to_owned(),
            None => String::new(),
        };
        Ok(Self {
            token,
            user_id: health.user_id,
            key_alias: fields.key_alias,
            team_id: health.team_id,
            proxy_admin,
        })
    }
}

pub fn proxy_admin_gate(caller: &Caller, detail: &str) -> Result<(), ServiceError> {
    if caller.proxy_admin {
        return Ok(());
    }
    Err(ServiceError::Rejection {
        status: StatusCode::FORBIDDEN,
        detail: detail.to_owned(),
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct KeyAlertFields {
    token: Option<SecretString>,
    key_alias: Option<String>,
}

fn key_alert_fields(key: Option<&UserAPIKeyAuth>) -> Result<KeyAlertFields, ServiceError> {
    let Some(key) = key else {
        return Ok(KeyAlertFields::default());
    };
    let value = serde_json::to_value(key).map_err(ServiceError::KeyFields)?;
    serde_json::from_value(value).map_err(ServiceError::KeyFields)
}

pub struct Probes<'probe> {
    pub mock: &'probe dyn MockRequests,
    pub checks: &'probe dyn IntegrationChecks,
    pub callbacks: &'probe dyn CallbackSource,
    pub alerts: AlertSetup<'probe>,
}

#[async_trait]
pub trait IntegrationChecks: Send + Sync {
    async fn datadog(&self) -> Result<IntegrationHealthStatus, ServiceTestError>;
    async fn datadog_metrics(&self) -> Result<IntegrationHealthStatus, ServiceTestError>;
    async fn arize(&self) -> Result<IntegrationHealthStatus, ServiceTestError>;
    async fn galileo(&self) -> Result<IntegrationHealthStatus, ServiceTestError>;
    async fn newrelic(&self) -> Result<IntegrationHealthStatus, ServiceTestError>;
    async fn sqs(&self) -> Result<IntegrationHealthStatus, ServiceTestError>;
    async fn pointfive(&self) -> Result<IntegrationHealthStatus, PointFiveSetupError>;
    async fn langfuse_auth_check(&self) -> Result<Option<String>, ServiceTestError>;
}

// the success-callback list as strings and as the registered callbacks behind them
pub trait CallbackSource: Send + Sync {
    fn success_callback_entries(&self) -> Vec<CallbackEntry>;
}

pub async fn dispatch(
    service: ServiceName,
    caller: &Caller,
    ports: &Probes<'_>,
) -> Result<Option<ServiceBody>, ServiceError> {
    let text = service.as_str();
    let listed = ports
        .callbacks
        .success_callback_entries()
        .iter()
        .any(|entry| get_callback_identifier(entry) == text);
    if listed && !matches!(service, ServiceName::Langfuse | ServiceName::PointFive) {
        return mock_request(service, ports).await;
    }
    branch(service, caller, ports).await
}

async fn branch(
    service: ServiceName,
    caller: &Caller,
    ports: &Probes<'_>,
) -> Result<Option<ServiceBody>, ServiceError> {
    match service {
        ServiceName::OpenMeter | ServiceName::Braintrust | ServiceName::GenericApi => {
            mock_request(service, ports).await
        }
        ServiceName::Datadog => Ok(Some(health_reply(
            ports.checks.datadog().await?,
            "Datadog is healthy",
        ))),
        ServiceName::DatadogMetrics => Ok(Some(health_reply(
            ports.checks.datadog_metrics().await?,
            "Datadog Metrics is healthy",
        ))),
        ServiceName::Arize => Ok(Some(health_reply(
            ports.checks.arize().await?,
            "Arize is healthy",
        ))),
        ServiceName::Galileo => Ok(Some(health_reply(
            ports.checks.galileo().await?,
            "Galileo is healthy",
        ))),
        ServiceName::Sqs => sqs_reply(ports.checks.sqs().await),
        ServiceName::NewRelic => {
            proxy_admin_gate(
                caller,
                "Only proxy admins can trigger the New Relic test event.",
            )?;
            newrelic(ports).await
        }
        ServiceName::PointFive => {
            proxy_admin_gate(
                caller,
                "Only proxy admins can trigger the PointFive liveness ping.",
            )?;
            pointfive(ports).await
        }
        ServiceName::Langfuse => langfuse(ports).await,
        ServiceName::Webhook => alert_tests::webhook(caller, &ports.alerts).await,
        ServiceName::Slack | ServiceName::SlackBudgetAlerts => {
            alert_tests::slack(service, &ports.alerts).await
        }
        ServiceName::MsTeams => alert_tests::ms_teams(&ports.alerts).await,
        ServiceName::Email => alert_tests::email(caller, &ports.alerts).await,
        ServiceName::Otel
        | ServiceName::CustomCallbackApi
        | ServiceName::Langsmith
        | ServiceName::LangfuseOtel
        | ServiceName::DatadogLlmObservability
        | ServiceName::Signoz => Ok(None),
    }
}

async fn mock_request(
    service: ServiceName,
    ports: &Probes<'_>,
) -> Result<Option<ServiceBody>, ServiceError> {
    ports.mock.send_mock_request(&MOCK_REQUEST).await?;
    let message = format!("Mock LLM request made - check {}.", service.as_str());
    Ok(Some(ServiceBody::Success(SuccessBody {
        status: "success",
        message,
    })))
}

fn health_reply(status: IntegrationHealthStatus, healthy: &str) -> ServiceBody {
    let message = if status.status == HealthState::Unhealthy {
        status.error_message
    } else {
        Some(healthy.to_owned())
    };
    ServiceBody::Health(HealthBody {
        status: status.status,
        message,
    })
}

fn sqs_reply(
    status: Result<IntegrationHealthStatus, ServiceTestError>,
) -> Result<Option<ServiceBody>, ServiceError> {
    let status = status?;
    Ok(Some(ServiceBody::Health(HealthBody {
        status: status.status,
        message: status.error_message,
    })))
}

async fn newrelic(ports: &Probes<'_>) -> Result<Option<ServiceBody>, ServiceError> {
    let status = ports.checks.newrelic().await?;
    Ok(Some(health_reply(
        status,
        "New Relic is healthy \u{2014} test event sent",
    )))
}

async fn pointfive(ports: &Probes<'_>) -> Result<Option<ServiceBody>, ServiceError> {
    match ports.checks.pointfive().await {
        Ok(status) => Ok(Some(pointfive_reply(status))),
        Err(setup) => Ok(Some(ServiceBody::Health(HealthBody {
            status: HealthState::Unhealthy,
            message: Some(setup.to_string()),
        }))),
    }
}

// an unhealthy check with no error text reads like a healthy one, and a healthy one ignores the text
fn pointfive_reply(status: IntegrationHealthStatus) -> ServiceBody {
    let unhealthy = status.status == HealthState::Unhealthy;
    let message = match status.error_message {
        Some(text) if !text.is_empty() && unhealthy => text,
        Some(_) | None => "PointFive is healthy".to_owned(),
    };
    ServiceBody::Health(HealthBody {
        status: status.status,
        message: Some(message),
    })
}

async fn langfuse(ports: &Probes<'_>) -> Result<Option<ServiceBody>, ServiceError> {
    if let Some(reason) = ports.checks.langfuse_auth_check().await? {
        return Err(ServiceError::LangfuseAuthCheck { reason });
    }
    mock_request(ServiceName::Langfuse, ports).await
}
