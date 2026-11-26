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

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::Display, strum::EnumString)]
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

// each accepted name runs one probe branch, so the dispatch matches the kind instead of restating the 22 names
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Mock,
    Langfuse,
    Datadog,
    DatadogMetrics,
    Arize,
    Galileo,
    Sqs,
    NewRelic,
    PointFive,
    Webhook,
    Slack,
    MsTeams,
    Email,
    NoReply,
}

impl ServiceName {
    #[must_use]
    pub const fn kind(self) -> Kind {
        match self {
            Self::OpenMeter | Self::Braintrust | Self::GenericApi => Kind::Mock,
            Self::Langfuse => Kind::Langfuse,
            Self::Datadog => Kind::Datadog,
            Self::DatadogMetrics => Kind::DatadogMetrics,
            Self::Arize => Kind::Arize,
            Self::Galileo => Kind::Galileo,
            Self::Sqs => Kind::Sqs,
            Self::NewRelic => Kind::NewRelic,
            Self::PointFive => Kind::PointFive,
            Self::Webhook => Kind::Webhook,
            Self::Slack | Self::SlackBudgetAlerts => Kind::Slack,
            Self::MsTeams => Kind::MsTeams,
            Self::Email => Kind::Email,
            Self::Otel
            | Self::CustomCallbackApi
            | Self::Langsmith
            | Self::LangfuseOtel
            | Self::DatadogLlmObservability
            | Self::Signoz => Kind::NoReply,
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
