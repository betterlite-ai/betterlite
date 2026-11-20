use std::sync::Arc;

use betterlite_config::sections::callback_settings::names::CallbackName;

use super::otel::shared::SharedOtel;
use super::{CatalogError, CatalogInputs};
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
        pub(super) fn build_logger(
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

builder_dispatch! {
    arms {
        Arize => super::arize,
        ArizePhoenix => super::arize_phoenix,
        LangfuseOtel => super::langfuse_otel,
        Langtrace => super::langtrace,
        Levo => super::levo,
        Logfire => super::logfire,
        Newrelic => super::newrelic,
        Otel => super::otel,
        Signoz => super::signoz,
        Traceloop => super::traceloop,
        WeaveOtel => super::weave_otel,
    }
    builders {
        Agentops => super::agentops,
        Argilla => super::argilla,
        Athina => super::athina,
        AwsSqs => super::aws_sqs,
        AzureSentinel => super::azure_sentinel,
        AzureStorage => super::azure_storage,
        Bitbucket => super::bitbucket,
        Braintrust => super::braintrust,
        Clickhouse => super::clickhouse,
        Cloudzero => super::cloudzero,
        DatadogCostManagement => super::datadog_cost_management,
        DatadogLlmObservability => super::datadog_llm_observability,
        DatadogMetrics => super::datadog_metrics,
        Deepeval => super::deepeval,
        Dotprompt => super::dotprompt,
        Dynamodb => super::dynamodb,
        Focus => super::focus,
        Galileo => super::galileo,
        GcsBucket => super::gcs_bucket,
        GcsPubsub => super::gcs_pubsub,
        Gitlab => super::gitlab,
        Greenscale => super::greenscale,
        Helicone => super::helicone,
        Humanloop => super::humanloop,
        Lago => super::lago,
        Langfuse => super::langfuse,
        Langsmith => super::langsmith,
        Literalai => super::literalai,
        Lunary => super::lunary,
        Mavvrik => super::mavvrik,
        Mlflow => super::mlflow,
        Openmeter => super::openmeter,
        Opik => super::opik,
        Pagerduty => super::pagerduty,
        Pointfive => super::pointfive,
        Posthog => super::posthog,
        Promptlayer => super::promptlayer,
        ResendEmail => super::resend_email,
        S3 => super::s3,
        S3V2 => super::s3_v2,
        SendgridEmail => super::sendgrid_email,
        Sentry => super::sentry,
        Slack => super::slack,
        SmtpEmail => super::smtp_email,
        Supabase => super::supabase,
        Vantage => super::vantage,
        Wandb => super::wandb,
        Zerobus => super::zerobus,
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
