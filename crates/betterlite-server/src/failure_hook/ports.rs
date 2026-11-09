use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::routes::CallType;
use betterlite_openai::openai_base_config::Object as JsonMap;
use rust_decimal::Decimal;
use thiserror::Error;

use super::lift::ModelCallDetails;
use crate::guardrails::callback_caps::errors::ProxyHookError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertType {
    LlmExceptions,
    LlmTooSlow,
    LlmRequestsHanging,
    BudgetAlerts,
    SpendReports,
    FailedTrackingSpend,
    UserSpendThresholds,
    UserSpendAnomalies,
    DbExceptions,
    DailyReports,
    CooldownDeployment,
    NewModelAdded,
    ModelDeprecationWarnings,
    OutageAlerts,
    RegionOutageAlerts,
    FallbackReports,
    NewVirtualKeyCreated,
    VirtualKeyUpdated,
    VirtualKeyDeleted,
    NewTeamCreated,
    TeamUpdated,
    TeamDeleted,
    NewInternalUserCreated,
    InternalUserUpdated,
    InternalUserDeleted,
    McpToolDescriptionBlocked,
    McpPinnedToolsChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::Display)]
pub enum AlertLevel {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetAlertKind {
    TokenBudget,
    UserBudget,
    SoftBudget,
    MaxBudgetAlert,
    TeamBudget,
    OrganizationBudget,
    ProxyBudget,
    ProjectedLimitExceeded,
    ProjectBudget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertingClient {
    Slack,
    Email,
    MsTeams,
    Sentry,
    Webhook,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestStatus {
    Success,
    Fail,
}

impl RequestStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Fail => "fail",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailureInput {
    Messages(Vec<JsonValue>),
    Prompt(String),
    Embedding(Vec<JsonValue>),
    Empty,
}

impl FailureInput {
    #[must_use]
    pub const fn detail_name(&self) -> &'static str {
        match self {
            Self::Messages(_) => "messages",
            Self::Prompt(_) => "prompt",
            Self::Embedding(_) => "input",
            Self::Empty => "",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallEnvironment {
    pub model: String,
    pub user: String,
    pub optional_params: JsonMap,
    pub litellm_params: JsonMap,
    pub custom_llm_provider: Option<String>,
}

// LiteLLM only needs to know whether counting succeeded, so the cause is not carried across the port
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("the request could not be counted")]
pub struct TokenCountError;

#[async_trait]
pub trait FailureTokenCounter: Send + Sync {
    async fn count_text(&self, model: &str, text: &str) -> Result<i64, TokenCountError>;
    async fn count_messages(
        &self,
        model: &str,
        messages: &[JsonValue],
    ) -> Result<i64, TokenCountError>;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListedDeployment {
    pub provider: Option<String>,
    pub model_info: JsonMap,
    pub model: String,
}

pub trait DeploymentCatalog: Send + Sync {
    fn get_model_list(&self, model_group: &str, team_id: Option<&str>) -> Vec<ListedDeployment>;
}

pub trait RouteCallTypes: Send + Sync {
    fn call_types_for(&self, route: &str) -> Option<Vec<CallType>>;
}

#[async_trait]
pub trait UsageCache: Send + Sync {
    async fn set_local(&self, key: &str, value: &str, ttl: Duration);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    #[default]
    Key,
    User,
    EndUser,
    Team,
    TeamMember,
    Organization,
    OrganizationMember,
    Project,
    Tag,
    Agent,
    ModelAccessGroup,
    Proxy,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallInfo {
    pub spend: Decimal,
    pub max_budget: Option<Decimal>,
    pub soft_budget: Option<Decimal>,
    pub token: Option<String>,
    pub customer_id: Option<String>,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub organization_id: Option<String>,
    pub user_email: Option<String>,
    pub key_alias: Option<String>,
    pub projected_exceeded_date: Option<String>,
    pub projected_spend: Option<Decimal>,
    pub event_group: EntityType,
    pub alert_emails: Option<Vec<String>>,
    pub max_budget_alert_emails: Option<BTreeMap<String, Vec<String>>>,
}

pub struct SlackAlert<'alert> {
    pub message: &'alert str,
    pub level: AlertLevel,
    pub alert_type: AlertType,
    pub user_info: Option<&'alert CallInfo>,
    pub alerting_metadata: &'alert JsonMap,
    pub langfuse_trace: Option<&'alert str>,
}

#[async_trait]
pub trait SlackAlerting: Send + Sync {
    fn alerting_configured(&self) -> bool;
    async fn failed_tracking_alert(&self, error_message: &str, failing_model: &str);
    async fn budget_alerts(&self, kind: BudgetAlertKind, user_info: &CallInfo);
    async fn send_alert(&self, alert: SlackAlert<'_>);
    async fn response_taking_too_long(&self, data: Option<JsonMap>);
}

#[async_trait]
pub trait EmailAlerting: Send + Sync {
    async fn budget_alerts(&self, kind: BudgetAlertKind, user_info: &CallInfo);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureService {
    Db,
}

#[async_trait]
pub trait ServiceLogging: Send + Sync {
    async fn async_service_failure_hook(
        &self,
        service: FailureService,
        duration: Duration,
        error: &str,
        call_type: &str,
    );
}

pub trait SentryCapture: Send + Sync {
    fn capture_message(&self, message: &str);
    fn capture_exception(&self, error: &ProxyHookError);
}

pub trait Clock: Send + Sync {
    fn now(&self) -> chrono::DateTime<chrono::FixedOffset>;
}

pub trait LoggingFactory: Send + Sync {
    // The created object starts with the route (or IGNORE_THIS) as its call type, like function_setup
    fn create_logging(
        &self,
        route: &str,
        call_id: &str,
        request_data: &JsonMap,
    ) -> Arc<dyn FailureLogging>;
}

#[async_trait]
pub trait FailureLogging: Send + Sync {
    fn details(&self) -> ModelCallDetails;
    fn call_type(&self) -> Option<String>;
    fn set_call_type(&self, call_type: CallType);
    fn record_input(&self, input: &FailureInput);
    fn mark_no_upstream_llm_call(&self);
    fn update_environment_variables(&self, environment: CallEnvironment);
    fn has_baseline_context(&self) -> bool;
    async fn invalidate_baseline_estimate(&self, reason: &str, completed: bool);
    async fn pre_call(&self, input: &FailureInput, api_key: &str);
    async fn async_failure_handler(&self, error: &ProxyHookError, traceback: &str);
    fn failure_handler(&self, error: &ProxyHookError, traceback: &str);
    async fn langfuse_trace_url(&self) -> Option<String>;
}
