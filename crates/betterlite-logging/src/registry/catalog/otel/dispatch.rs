use std::sync::Arc;

use betterlite_config::sections::callback_settings::names::CallbackName;

use super::super::{CatalogError, CatalogInputs};
use super::shared::SharedOtel;
use crate::callback_logger::CallbackLogger;
use crate::otel::logger_runtime::LoggerRuntime;

// LiteLLM appends every logger it builds; a V2 logger already appended by maybe_construct_otel_v2 stays once
fn push_once(list: &mut Vec<Arc<dyn CallbackLogger>>, logger: &Arc<dyn CallbackLogger>) {
    if !list.iter().any(|held| Arc::ptr_eq(held, logger)) {
        list.push(Arc::clone(logger));
    }
}

macro_rules! builder_dispatch {
    (
        arms { $($arm:ident => $arm_module:ident),+ $(,)? }
        builders { $($variant:ident => $module:ident),+ $(,)? }
        refused { $($refused:ident),+ $(,)? }
    ) => {
        pub(crate) fn build_logger(
            callback: CallbackName,
            inputs: &CatalogInputs,
            runtime: Option<&LoggerRuntime>,
            shared: &SharedOtel,
            in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
        ) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
            match callback {
                $(
                    CallbackName::$arm => {
                        let logger = $arm_module::build(inputs, runtime, shared, in_memory)?;
                        push_once(in_memory, &logger);
                        Ok(logger)
                    }
                )+
                $(
                    CallbackName::$variant => {
                        let logger = $module::build(inputs)?;
                        push_once(in_memory, &logger);
                        Ok(logger)
                    }
                )+
                $(CallbackName::$refused)|+ => Err(CatalogError::Unavailable {
                    name: callback.as_str().to_owned(),
                }),
            }
        }
    };
}

// This table lives here because catalog.rs would pass its 300-line limit with it inline
builder_dispatch! {
    arms {
        Arize => super::super::arize,
        ArizePhoenix => super::super::arize_phoenix,
        LangfuseOtel => super::super::langfuse_otel,
        Langtrace => super::super::langtrace,
        Levo => super::super::levo,
        Logfire => super::super::logfire,
        Newrelic => super::super::newrelic,
        Otel => super::super::otel,
        Signoz => super::super::signoz,
        Traceloop => super::super::traceloop,
        WeaveOtel => super::super::weave_otel,
    }
    builders {
        Agentops => super::super::agentops,
        Argilla => super::super::argilla,
        Athina => super::super::athina,
        AwsSqs => super::super::aws_sqs,
        AzureSentinel => super::super::azure_sentinel,
        AzureStorage => super::super::azure_storage,
        Bitbucket => super::super::bitbucket,
        Braintrust => super::super::braintrust,
        Clickhouse => super::super::clickhouse,
        Cloudzero => super::super::cloudzero,
        DatadogCostManagement => super::super::datadog_cost_management,
        DatadogLlmObservability => super::super::datadog_llm_observability,
        DatadogMetrics => super::super::datadog_metrics,
        Deepeval => super::super::deepeval,
        Dotprompt => super::super::dotprompt,
        Dynamodb => super::super::dynamodb,
        Focus => super::super::focus,
        Galileo => super::super::galileo,
        GcsBucket => super::super::gcs_bucket,
        GcsPubsub => super::super::gcs_pubsub,
        Gitlab => super::super::gitlab,
        Greenscale => super::super::greenscale,
        Helicone => super::super::helicone,
        Humanloop => super::super::humanloop,
        Lago => super::super::lago,
        Langfuse => super::super::langfuse,
        Langsmith => super::super::langsmith,
        Literalai => super::super::literalai,
        Lunary => super::super::lunary,
        Mavvrik => super::super::mavvrik,
        Mlflow => super::super::mlflow,
        Openmeter => super::super::openmeter,
        Opik => super::super::opik,
        Pagerduty => super::super::pagerduty,
        Pointfive => super::super::pointfive,
        Posthog => super::super::posthog,
        Promptlayer => super::super::promptlayer,
        ResendEmail => super::super::resend_email,
        S3 => super::super::s3,
        S3V2 => super::super::s3_v2,
        SendgridEmail => super::super::sendgrid_email,
        Sentry => super::super::sentry,
        Slack => super::super::slack,
        SmtpEmail => super::super::smtp_email,
        Supabase => super::super::supabase,
        Vantage => super::super::vantage,
        Wandb => super::super::wandb,
        Zerobus => super::super::zerobus,
    }
    refused {
        Prometheus,
        Datadog,
        LitellmAgent,
        GenericApi,
        SampleCallback,
        Rubrik,
        Sumologic,
        QualifireEval,
        Ramp,
        DynamicRateLimiter,
        DynamicRateLimiterV3,
        AnthropicCacheControlHook,
        VectorStorePreCallHook,
        CompressionInterception,
    }
}
