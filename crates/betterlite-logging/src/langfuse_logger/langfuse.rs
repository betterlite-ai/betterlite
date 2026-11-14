use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio_util::task::TaskTracker;

use crate::fields::payload::StandardLoggingPayload;
use crate::langfuse_assembly::{
    AssemblyRuntime, AssemblySettings, Masker, assemble_generation_trace,
};
use crate::langfuse_channels::{
    CHANNEL_RETIRE_GRACE, DEFAULT_FLUSH_TIMEOUT_MILLIS, LangfuseChannelRequest, LangfuseTracing,
    acquire_langfuse_tracing, release_langfuse_tracing,
};
use crate::langfuse_exporter::exporter::build_span_exporter;
use crate::langfuse_exporter::http_transport::HttpTransport;
use crate::langfuse_observations::identity::Timestamp;
use crate::langfuse_observations::requested_ids::RequestedIdGenerator;
use crate::langfuse_projections::content::ResponseObj;
use crate::langfuse_projections::input_output::{ContentRequest, get_input_output_content};
use crate::langfuse_rest::client::LangfuseApiClient;
use crate::langfuse_settings::{
    Credentials, export, mock_mode, resolve_credentials, resolve_environment,
    warn_if_upstream_configured,
};

use self::event::{EventFragments, event_fragments, generation_trace};
use super::budget::LangfuseClientBudget;
use super::{
    AssemblyDefaults, LangfuseLoggerError, LangfuseLoggerRequest, LangfuseServices,
    channel_request, client_config,
};

const MOCK_PROJECT_ID: &str = "mock-project-id";

// litellm.types.integrations.langfuse.LangfuseLoggedEvent
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct LangfuseLoggedEvent {
    pub trace_id: Option<String>,
    pub generation_id: Option<String>,
}

// litellm.types.integrations.langfuse.LangfuseLoggingConfig; the secret keeps its wrapper
#[derive(Clone, Debug, Default)]
pub struct LangfuseLoggingConfig {
    pub langfuse_secret: Option<SecretString>,
    pub langfuse_public_key: Option<String>,
    pub langfuse_host: Option<String>,
    pub langfuse_environment: Option<String>,
}

// the kwargs LangFuseLogger.log_event_on_langfuse reads for one success or failure event
pub struct LangfuseCall<'call> {
    pub payload: &'call StandardLoggingPayload,
    pub litellm_params: Option<&'call Json>,
    pub messages: Option<&'call Json>,
    pub optional_params: &'call IndexMap<String, Json>,
    pub response: Option<&'call Json>,
    pub response_object: Option<ResponseObj<'call>>,
    pub start_time: Option<Timestamp>,
    pub end_time: Option<Timestamp>,
    pub completion_start_time: Option<Timestamp>,
    pub level: &'call str,
    pub status_message: Option<&'call str>,
    pub litellm_trace_id: Option<&'call str>,
    pub vertex_location: Option<&'call str>,
    pub aws_region_name: Option<&'call str>,
    pub preset_cache_key: Option<&'call str>,
    pub input: Option<&'call Json>,
    pub masker: Option<Masker<'call>>,
}

// litellm.integrations.langfuse.langfuse.LangFuseLogger, holding one REST client and one export channel
pub struct LangFuseLogger {
    tracing: LangfuseTracing,
    api_client: LangfuseApiClient,
    pub project_id: Option<String>,
    budget: LangfuseClientBudget,
    tasks: TaskTracker,
    runtime: Handle,
    environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    assembly: AssemblyDefaults,
    stopped: AtomicBool,
}

impl LangFuseLogger {
    pub async fn new(
        services: &LangfuseServices,
        request: &LangfuseLoggerRequest<'_>,
    ) -> Result<Self, LangfuseLoggerError> {
        export::check_prompt_cache_ttl(&services.settings)?;
        let credentials = resolve_credentials(&services.settings, request.credentials);
        let environment = resolve_environment(&services.settings, request.environment)?;
        let mock = mock_mode(&services.settings)?;
        let flush_interval = export::flush_interval(&services.settings, request.flush_interval);
        let reservation = services.budget.reserve()?;
        let channel = channel_request(
            &services.settings,
            &credentials,
            environment,
            flush_interval,
            mock,
        );
        let (api_client, tracing) = initialize(services, &credentials, &channel).await?;
        reservation.commit();
        let project_id = resolve_project_id(&api_client, mock).await;
        warn_if_upstream_configured(&services.settings);
        Ok(Self {
            tracing,
            api_client,
            project_id,
            budget: services.budget.clone(),
            tasks: services.tasks.clone(),
            runtime: services.runtime.clone(),
            environment: Arc::clone(&services.environment),
            assembly: services.assembly.clone(),
            stopped: AtomicBool::new(false),
        })
    }

    // LiteLLM's flush discards the export result; betterlite keeps it for its caller
    #[must_use]
    pub async fn flush(&self) -> bool {
        let flushed = self.tracing.flush(DEFAULT_FLUSH_TIMEOUT_MILLIS).await;
        if !flushed {
            tracing::debug!("the langfuse export channel did not drain before the flush deadline");
        }
        flushed
    }

    pub fn stop(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        self.budget.release();
        let tracing = self.tracing.clone();
        let _release = self.tasks.spawn_on(
            async move { release_langfuse_tracing(&tracing, CHANNEL_RETIRE_GRACE).await },
            &self.runtime,
        );
    }

    #[must_use]
    pub async fn log_event_on_langfuse(&self, call: &LangfuseCall<'_>) -> LangfuseLoggedEvent {
        let EventFragments {
            metadata,
            prompt,
            optional_params,
        } = event_fragments(call);
        let (input, output) = get_input_output_content(
            &prompt,
            call.response_object.as_ref(),
            &ContentRequest {
                call_type: Some(call.payload.call_type.as_str()),
                input: call.input,
                level: call.level,
                status_message: call.status_message,
            },
        );
        let trace = generation_trace(
            call,
            &metadata,
            input.as_ref(),
            output.as_ref(),
            optional_params,
        );
        let generator = RequestedIdGenerator::default();
        let runtime = AssemblyRuntime {
            tracer: &self.tracing.tracer,
            id_generator: &generator,
            prompt_client: &self.api_client,
            environment: self.environment.as_ref(),
        };
        let settings = AssemblySettings {
            default_tags: &self.assembly.default_tags,
            update_trace_keys: self.assembly.update_trace_keys,
            redact_user_api_key_info: self.assembly.redact_user_api_key_info,
        };
        let (trace_id, generation_id) =
            assemble_generation_trace(&trace, &runtime, &settings).await;
        LangfuseLoggedEvent {
            trace_id: trace_id.as_ref().map(ToString::to_string),
            generation_id,
        }
    }
}

// safe_init_langfuse_client: acquire the channel, then build the client, giving the channel back on failure
async fn initialize(
    services: &LangfuseServices,
    credentials: &Credentials,
    channel: &LangfuseChannelRequest,
) -> Result<(LangfuseApiClient, LangfuseTracing), LangfuseLoggerError> {
    let settings = &services.settings;
    let environment = Arc::clone(&services.environment);
    let exporter_credentials = credentials.clone();
    let http = services.http.clone();
    let tracing = acquire_langfuse_tracing(
        settings,
        services.environment.as_ref(),
        &services.runtime,
        channel,
        move || {
            build_span_exporter(
                settings,
                environment.as_ref(),
                &exporter_credentials,
                HttpTransport { client: http },
            )
        },
    )?;
    let config = match client_config(settings, credentials) {
        Ok(config) => config,
        Err(error) => {
            release_langfuse_tracing(&tracing, Duration::ZERO).await;
            return Err(error);
        }
    };
    match services.clients.build(config) {
        Ok(api_client) => Ok((api_client, tracing)),
        Err(error) => {
            release_langfuse_tracing(&tracing, Duration::ZERO).await;
            Err(LangfuseLoggerError::Client(error))
        }
    }
}

// LiteLLM writes LANGFUSE_PROJECT_ID into the process environment; betterlite keeps it on the logger
async fn resolve_project_id(api_client: &LangfuseApiClient, mock: bool) -> Option<String> {
    if mock {
        tracing::debug!("Langfuse Mock: Using mock project ID");
        return Some(MOCK_PROJECT_ID.to_owned());
    }
    match api_client.project_id().await {
        Ok(project_id) => project_id,
        Err(error) => {
            tracing::debug!(
                "Langfuse project id unavailable, alerting links will omit it: {error}"
            );
            None
        }
    }
}

mod event;
mod tests;
