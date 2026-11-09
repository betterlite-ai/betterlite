use rust_decimal::Decimal;
use serde::Deserialize;

pub const DEFAULT_DIGEST_INTERVAL: u32 = 86_400;

#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    serde::Serialize,
    strum::Display,
    strum::EnumCount,
    strum::EnumIter,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
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

// LiteLLM's DEFAULT_ALERT_TYPES: user spend anomalies and the management events are opt-in
pub const DEFAULT_ALERT_TYPES: [AlertType; 17] = [
    AlertType::LlmExceptions,
    AlertType::LlmTooSlow,
    AlertType::LlmRequestsHanging,
    AlertType::BudgetAlerts,
    AlertType::SpendReports,
    AlertType::FailedTrackingSpend,
    AlertType::UserSpendThresholds,
    AlertType::DbExceptions,
    AlertType::DailyReports,
    AlertType::CooldownDeployment,
    AlertType::NewModelAdded,
    AlertType::ModelDeprecationWarnings,
    AlertType::OutageAlerts,
    AlertType::RegionOutageAlerts,
    AlertType::FallbackReports,
    AlertType::McpToolDescriptionBlocked,
    AlertType::McpPinnedToolsChanged,
];

pub const DEFAULT_DAILY_REPORT_FREQUENCY: u32 = 43_200;
const DEFAULT_REPORT_CHECK_INTERVAL: u32 = 300;
const DEFAULT_BUDGET_ALERT_TTL: u32 = 86_400;
const DEFAULT_OUTAGE_ALERT_TTL: u32 = 60;
const DEFAULT_REGION_OUTAGE_TTL: u32 = 60;
const DEFAULT_MINOR_OUTAGE_THRESHOLD: u32 = 5;
const DEFAULT_MAJOR_OUTAGE_THRESHOLD: u32 = 10;
const DEFAULT_MAX_OUTAGE_LIST_SIZE: u32 = 10;
const DEFAULT_ANOMALY_MULTIPLIER: Decimal = Decimal::from_parts(3, 0, 0, false, 0);
const DEFAULT_ANOMALY_BASELINE_DAYS: u32 = 7;
const DEFAULT_ANOMALY_MIN_SPEND: Decimal = Decimal::from_parts(10, 0, 0, false, 0);
const DEFAULT_SPEND_CHECK_INTERVAL: u32 = 3600;

#[derive(Clone, Copy, Debug)]
pub(super) struct CountField {
    pub(super) name: &'static str,
    pub(super) default: u32,
    pub(super) minimum: u32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct OptionalMoneyField {
    pub(super) name: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RequiredMoneyField {
    pub(super) name: &'static str,
    pub(super) default: Decimal,
}

pub(super) const REPORT_CHECK_INTERVAL: CountField = CountField {
    name: "report_check_interval",
    default: DEFAULT_REPORT_CHECK_INTERVAL,
    minimum: 0,
};

pub(super) const BUDGET_ALERT_TTL: CountField = CountField {
    name: "budget_alert_ttl",
    default: DEFAULT_BUDGET_ALERT_TTL,
    minimum: 0,
};

pub(super) const OUTAGE_ALERT_TTL: CountField = CountField {
    name: "outage_alert_ttl",
    default: DEFAULT_OUTAGE_ALERT_TTL,
    minimum: 0,
};

pub(super) const REGION_OUTAGE_TTL: CountField = CountField {
    name: "region_outage_alert_ttl",
    default: DEFAULT_REGION_OUTAGE_TTL,
    minimum: 0,
};

pub(super) const MINOR_OUTAGE_THRESHOLD: CountField = CountField {
    name: "minor_outage_alert_threshold",
    default: DEFAULT_MINOR_OUTAGE_THRESHOLD,
    minimum: 0,
};

pub(super) const MAJOR_OUTAGE_THRESHOLD: CountField = CountField {
    name: "major_outage_alert_threshold",
    default: DEFAULT_MAJOR_OUTAGE_THRESHOLD,
    minimum: 0,
};

pub(super) const MAX_OUTAGE_LIST_SIZE: CountField = CountField {
    name: "max_outage_alert_list_size",
    default: DEFAULT_MAX_OUTAGE_LIST_SIZE,
    minimum: 0,
};

pub(super) const ANOMALY_BASELINE_DAYS: CountField = CountField {
    name: "spend_anomaly_baseline_days",
    default: DEFAULT_ANOMALY_BASELINE_DAYS,
    minimum: 1,
};

pub(super) const SPEND_CHECK_INTERVAL: CountField = CountField {
    name: "user_spend_check_interval",
    default: DEFAULT_SPEND_CHECK_INTERVAL,
    minimum: 60,
};

pub(super) const DAILY_SPEND_THRESHOLD: OptionalMoneyField = OptionalMoneyField {
    name: "daily_spend_per_user_threshold",
};

pub(super) const MONTHLY_SPEND_THRESHOLD: OptionalMoneyField = OptionalMoneyField {
    name: "monthly_spend_per_user_threshold",
};

pub(super) const ANOMALY_MULTIPLIER: RequiredMoneyField = RequiredMoneyField {
    name: "spend_anomaly_multiplier",
    default: DEFAULT_ANOMALY_MULTIPLIER,
};

pub(super) const ANOMALY_MIN_SPEND: RequiredMoneyField = RequiredMoneyField {
    name: "spend_anomaly_min_spend",
    default: DEFAULT_ANOMALY_MIN_SPEND,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlackAlertingArgs {
    pub daily_report_frequency: u32,
    pub report_check_interval: u32,
    pub budget_alert_ttl: u32,
    pub outage_alert_ttl: u32,
    pub region_outage_alert_ttl: u32,
    pub minor_outage_alert_threshold: u32,
    pub major_outage_alert_threshold: u32,
    pub max_outage_alert_list_size: u32,
    pub log_to_console: bool,
    pub daily_user_spend_threshold: Option<Decimal>,
    pub monthly_user_spend_threshold: Option<Decimal>,
    pub spend_anomaly_multiplier: Decimal,
    pub spend_anomaly_baseline_days: u32,
    pub spend_anomaly_min_spend: Decimal,
    pub user_spend_check_interval: u32,
}

impl Default for SlackAlertingArgs {
    fn default() -> Self {
        Self {
            daily_report_frequency: DEFAULT_DAILY_REPORT_FREQUENCY,
            report_check_interval: REPORT_CHECK_INTERVAL.default,
            budget_alert_ttl: BUDGET_ALERT_TTL.default,
            outage_alert_ttl: OUTAGE_ALERT_TTL.default,
            region_outage_alert_ttl: REGION_OUTAGE_TTL.default,
            minor_outage_alert_threshold: MINOR_OUTAGE_THRESHOLD.default,
            major_outage_alert_threshold: MAJOR_OUTAGE_THRESHOLD.default,
            max_outage_alert_list_size: MAX_OUTAGE_LIST_SIZE.default,
            log_to_console: false,
            daily_user_spend_threshold: None,
            monthly_user_spend_threshold: None,
            spend_anomaly_multiplier: ANOMALY_MULTIPLIER.default,
            spend_anomaly_baseline_days: ANOMALY_BASELINE_DAYS.default,
            spend_anomaly_min_spend: ANOMALY_MIN_SPEND.default,
            user_spend_check_interval: SPEND_CHECK_INTERVAL.default,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlertTypeConfig {
    pub digest: bool,
    pub digest_interval: u32,
}

impl Default for AlertTypeConfig {
    fn default() -> Self {
        Self {
            digest: false,
            digest_interval: DEFAULT_DIGEST_INTERVAL,
        }
    }
}
