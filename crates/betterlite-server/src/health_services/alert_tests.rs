use async_trait::async_trait;
use axum::http::StatusCode;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::AlertLevel;
use betterlite_logging::budget_email::event::{EntityType, WebhookEvent, WebhookEventType};
use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::failure_hook::ports::Clock;
use crate::health_services::service_name::ServiceName;
use crate::health_services::service_probes::{
    Caller, ServiceError, ServiceTestError, proxy_admin_gate,
};
use crate::health_services::service_routes::{ServiceBody, SlackBody, SuccessBody};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UserBudgetAlert {
    pub token: String,
    pub user_id: Option<String>,
    pub key_alias: Option<String>,
    pub team_id: Option<String>,
}

impl From<&Caller> for UserBudgetAlert {
    fn from(caller: &Caller) -> Self {
        Self {
            token: caller.token.clone(),
            user_id: caller.user_id.clone(),
            key_alias: caller.key_alias.clone(),
            team_id: caller.team_id.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamsDelivery {
    pub status_code: u16,
    pub body: String,
}

pub struct AlertSetup<'setup> {
    pub actions: &'setup dyn AlertActions,
    pub clock: &'setup dyn Clock,
    pub alerting: &'setup [String],
    pub test_email_address: Option<&'setup str>,
}

#[async_trait]
pub trait AlertActions: Send + Sync {
    async fn send_user_budget_test(&self, alert: &UserBudgetAlert) -> Result<(), ServiceTestError>;

    async fn send_alert(
        &self,
        message: &str,
        level: AlertLevel,
        alert_type: AlertType,
    ) -> Result<(), ServiceTestError>;

    async fn send_test_email(&self, event: &WebhookEvent) -> Result<bool, ServiceTestError>;

    async fn post_teams(
        &self,
        url: &SecretString,
        text: &str,
    ) -> Result<TeamsDelivery, ServiceTestError>;

    fn webhook_alert_types(&self) -> Option<Vec<AlertType>>;

    fn active_alert_types(&self) -> Option<Vec<AlertType>>;

    fn ms_teams_webhook_url(&self) -> Option<SecretString>;

    fn has_database(&self) -> bool;

    fn send_monthly_spend_report(&self);

    fn send_weekly_spend_report(&self);
}

pub async fn webhook(
    caller: &Caller,
    setup: &AlertSetup<'_>,
) -> Result<Option<ServiceBody>, ServiceError> {
    proxy_admin_gate(
        caller,
        "Only proxy admins can trigger the webhook test alert.",
    )?;
    setup
        .actions
        .send_user_budget_test(&UserBudgetAlert::from(caller))
        .await?;
    Ok(None)
}

pub async fn slack(
    service: ServiceName,
    setup: &AlertSetup<'_>,
) -> Result<Option<ServiceBody>, ServiceError> {
    if !setup.alerting.iter().any(|entry| entry == "slack") {
        return Err(not_configured(&service.to_string()));
    }
    let active = setup.actions.active_alert_types();
    send_slack_alerts(setup, active.as_deref()).await?;
    if setup.actions.has_database() {
        setup.actions.send_monthly_spend_report();
        setup.actions.send_weekly_spend_report();
    }
    let alert_types = active
        .unwrap_or_default()
        .iter()
        .map(ToString::to_string)
        .collect();
    Ok(Some(ServiceBody::Slack(SlackBody {
        status: "success",
        alert_types,
        message: "Mock Slack Alert sent, verify Slack Alert Received on your channel",
    })))
}

async fn send_slack_alerts(
    setup: &AlertSetup<'_>,
    active: Option<&[AlertType]>,
) -> Result<(), ServiceError> {
    match setup.actions.webhook_alert_types() {
        Some(alert_types) => {
            for alert_type in alert_types {
                if active.is_some_and(|types| !types.contains(&alert_type)) {
                    continue;
                }
                send_slack_alert(setup, slack_test_message(alert_type), alert_type).await?;
            }
        }
        None => {
            send_slack_alert(
                setup,
                "This is a test slack alert message",
                AlertType::BudgetAlerts,
            )
            .await?;
        }
    }
    Ok(())
}

async fn send_slack_alert(
    setup: &AlertSetup<'_>,
    message: &str,
    alert_type: AlertType,
) -> Result<(), ServiceError> {
    setup
        .actions
        .send_alert(message, AlertLevel::Low, alert_type)
        .await?;
    Ok(())
}

fn not_configured(service: &str) -> ServiceError {
    ServiceError::Rejection {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        detail: format!(
            "\"{service}\" not in proxy config: general_settings. Unable to test this."
        ),
    }
}

pub async fn ms_teams(setup: &AlertSetup<'_>) -> Result<Option<ServiceBody>, ServiceError> {
    if !setup.alerting.iter().any(|entry| entry == "ms_teams") {
        return Err(not_configured("ms_teams"));
    }
    let Some(url) = setup.actions.ms_teams_webhook_url() else {
        return Err(ServiceError::Rejection {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            detail: "MS_TEAMS_WEBHOOK_URL not set. Unable to test this.".to_owned(),
        });
    };
    let alert_type = AlertType::BudgetAlerts;
    let level = AlertLevel::Low.as_str();
    let timestamp = setup.clock.now().format("%H:%M:%S");
    let message = format!(
        "Alert type: `{alert_type}`\nLevel: `{level}`\nTimestamp: `{timestamp}`\n\nMessage: This is a test MS Teams alert message"
    );
    let delivery = setup.actions.post_teams(&url, &message).await?;
    if delivery.status_code >= 400 {
        let TeamsDelivery { status_code, body } = delivery;
        return Err(ServiceError::Rejection {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            detail: format!("MS Teams webhook returned status {status_code}: {body}"),
        });
    }
    Ok(Some(ServiceBody::Success(SuccessBody {
        status: "success",
        message: "Mock MS Teams Alert sent, verify MS Teams Alert Received in your channel"
            .to_owned(),
    })))
}

pub async fn email(
    caller: &Caller,
    setup: &AlertSetup<'_>,
) -> Result<Option<ServiceBody>, ServiceError> {
    let event = WebhookEvent {
        event: WebhookEventType::KeyCreated,
        event_message: "Test Email Alert".to_owned(),
        event_group: EntityType::Key,
        spend: Decimal::ZERO,
        max_budget: Some(Decimal::ZERO),
        soft_budget: None,
        token: Some(SecretString::from(caller.token.clone())),
        customer_id: None,
        user_id: caller.user_id.clone(),
        team_id: caller.team_id.clone(),
        team_alias: None,
        organization_id: None,
        user_email: setup.test_email_address.map(str::to_owned),
        key_alias: Some(
            "Email Test key (This is only a test alert key. DO NOT USE THIS IN PRODUCTION.)"
                .to_owned(),
        ),
        projected_exceeded_date: None,
        projected_spend: None,
        alert_emails: None,
        max_budget_alert_emails: None,
    };
    let _sent = setup.actions.send_test_email(&event).await?;
    Ok(Some(ServiceBody::Success(SuccessBody {
        status: "success",
        message: "Mock Email Alert sent, verify Email Alert Received".to_owned(),
    })))
}

const fn slack_test_message(alert_type: AlertType) -> &'static str {
    match alert_type {
        AlertType::LlmExceptions => "LLM Exception test alert",
        AlertType::LlmTooSlow => "LLM Too Slow test alert",
        AlertType::LlmRequestsHanging => "LLM Requests Hanging test alert",
        AlertType::DbExceptions => "DB Exception test alert",
        AlertType::OutageAlerts => "Outage Alert Exception test alert",
        AlertType::DailyReports => "Daily Reports test alert",
        AlertType::BudgetAlerts
        | AlertType::SpendReports
        | AlertType::FailedTrackingSpend
        | AlertType::UserSpendThresholds
        | AlertType::UserSpendAnomalies
        | AlertType::CooldownDeployment
        | AlertType::NewModelAdded
        | AlertType::ModelDeprecationWarnings
        | AlertType::RegionOutageAlerts
        | AlertType::FallbackReports
        | AlertType::NewVirtualKeyCreated
        | AlertType::VirtualKeyUpdated
        | AlertType::VirtualKeyDeleted
        | AlertType::NewTeamCreated
        | AlertType::TeamUpdated
        | AlertType::TeamDeleted
        | AlertType::NewInternalUserCreated
        | AlertType::InternalUserUpdated
        | AlertType::InternalUserDeleted
        | AlertType::McpToolDescriptionBlocked
        | AlertType::McpPinnedToolsChanged => "Budget Alert test alert",
    }
}
