macro_rules! callback_names {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum CallbackName {
            $($variant),+
        }

        impl CallbackName {
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            #[must_use]
            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|callback| callback.as_str() == name)
            }

            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }
        }
    };
}

// one table per name, since a hand-written 73 arm match breaks the 40 line function limit
callback_names! {
    Lago => "lago",
    Openmeter => "openmeter",
    Logfire => "logfire",
    Literalai => "literalai",
    LitellmAgent => "litellm_agent",
    DynamicRateLimiter => "dynamic_rate_limiter",
    DynamicRateLimiterV3 => "dynamic_rate_limiter_v3",
    Langsmith => "langsmith",
    Prometheus => "prometheus",
    Otel => "otel",
    Datadog => "datadog",
    DatadogMetrics => "datadog_metrics",
    DatadogLlmObservability => "datadog_llm_observability",
    Galileo => "galileo",
    Braintrust => "braintrust",
    Arize => "arize",
    ArizePhoenix => "arize_phoenix",
    Langtrace => "langtrace",
    GcsBucket => "gcs_bucket",
    AzureStorage => "azure_storage",
    Opik => "opik",
    Argilla => "argilla",
    Mlflow => "mlflow",
    Langfuse => "langfuse",
    LangfuseOtel => "langfuse_otel",
    WeaveOtel => "weave_otel",
    Pagerduty => "pagerduty",
    Humanloop => "humanloop",
    AzureSentinel => "azure_sentinel",
    GcsPubsub => "gcs_pubsub",
    Agentops => "agentops",
    AnthropicCacheControlHook => "anthropic_cache_control_hook",
    GenericApi => "generic_api",
    ResendEmail => "resend_email",
    SendgridEmail => "sendgrid_email",
    SmtpEmail => "smtp_email",
    Deepeval => "deepeval",
    S3V2 => "s3_v2",
    Clickhouse => "clickhouse",
    Pointfive => "pointfive",
    Zerobus => "zerobus",
    AwsSqs => "aws_sqs",
    VectorStorePreCallHook => "vector_store_pre_call_hook",
    Dotprompt => "dotprompt",
    Bitbucket => "bitbucket",
    Gitlab => "gitlab",
    Cloudzero => "cloudzero",
    Focus => "focus",
    Mavvrik => "mavvrik",
    Vantage => "vantage",
    Posthog => "posthog",
    Levo => "levo",
    CompressionInterception => "compression_interception",
    Newrelic => "newrelic",
    Signoz => "signoz",
    Sentry => "sentry",
    Slack => "slack",
    Traceloop => "traceloop",
    Athina => "athina",
    Helicone => "helicone",
    Lunary => "lunary",
    Promptlayer => "promptlayer",
    Dynamodb => "dynamodb",
    S3 => "s3",
    Wandb => "wandb",
    Supabase => "supabase",
    Greenscale => "greenscale",
    DatadogCostManagement => "datadog_cost_management",
    SampleCallback => "sample_callback",
    Rubrik => "rubrik",
    Sumologic => "sumologic",
    QualifireEval => "qualifire_eval",
    Ramp => "ramp",
}
