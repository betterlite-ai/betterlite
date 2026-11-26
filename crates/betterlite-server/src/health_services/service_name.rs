pub const SERVICE_NAMES: [&str; 22] = [
    "slack_budget_alerts",
    "email",
    "langfuse",
    "langfuse_otel",
    "slack",
    "ms_teams",
    "openmeter",
    "webhook",
    "braintrust",
    "otel",
    "custom_callback_api",
    "langsmith",
    "datadog",
    "datadog_metrics",
    "datadog_llm_observability",
    "generic_api",
    "arize",
    "galileo",
    "newrelic",
    "pointfive",
    "signoz",
    "sqs",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString)]
pub enum ServiceName {
    #[strum(serialize = "slack_budget_alerts")]
    SlackBudgetAlerts,
    #[strum(serialize = "email")]
    Email,
    #[strum(serialize = "langfuse")]
    Langfuse,
    #[strum(serialize = "langfuse_otel")]
    LangfuseOtel,
    #[strum(serialize = "slack")]
    Slack,
    #[strum(serialize = "ms_teams")]
    MsTeams,
    #[strum(serialize = "openmeter")]
    OpenMeter,
    #[strum(serialize = "webhook")]
    Webhook,
    #[strum(serialize = "braintrust")]
    Braintrust,
    #[strum(serialize = "otel")]
    Otel,
    #[strum(serialize = "custom_callback_api")]
    CustomCallbackApi,
    #[strum(serialize = "langsmith")]
    Langsmith,
    #[strum(serialize = "datadog")]
    Datadog,
    #[strum(serialize = "datadog_metrics")]
    DatadogMetrics,
    #[strum(serialize = "datadog_llm_observability")]
    DatadogLlmObservability,
    #[strum(serialize = "generic_api")]
    GenericApi,
    #[strum(serialize = "arize")]
    Arize,
    #[strum(serialize = "galileo")]
    Galileo,
    #[strum(serialize = "newrelic")]
    NewRelic,
    #[strum(serialize = "pointfive")]
    PointFive,
    #[strum(serialize = "signoz")]
    Signoz,
    #[strum(serialize = "sqs")]
    Sqs,
}

impl ServiceName {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SlackBudgetAlerts => "slack_budget_alerts",
            Self::Email => "email",
            Self::Langfuse => "langfuse",
            Self::LangfuseOtel => "langfuse_otel",
            Self::Slack => "slack",
            Self::MsTeams => "ms_teams",
            Self::OpenMeter => "openmeter",
            Self::Webhook => "webhook",
            Self::Braintrust => "braintrust",
            Self::Otel => "otel",
            Self::CustomCallbackApi => "custom_callback_api",
            Self::Langsmith => "langsmith",
            Self::Datadog => "datadog",
            Self::DatadogMetrics => "datadog_metrics",
            Self::DatadogLlmObservability => "datadog_llm_observability",
            Self::GenericApi => "generic_api",
            Self::Arize => "arize",
            Self::Galileo => "galileo",
            Self::NewRelic => "newrelic",
            Self::PointFive => "pointfive",
            Self::Signoz => "signoz",
            Self::Sqs => "sqs",
        }
    }
}

// LiteLLM prints the names of its typing literal; betterlite prints the same names as a Python list
#[must_use]
pub fn unknown_service_message(service: &str) -> String {
    let quoted: Vec<String> = SERVICE_NAMES
        .iter()
        .map(|name| format!("'{name}'"))
        .collect();
    format!(
        "Service must be in list. Service={service} not in [{}]",
        quoted.join(", ")
    )
}
