use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::assistant_run_sdk::call::{RunFlag, RunParams, RunThreadCall};
use betterlite_core::api::assistant_run_sdk::provider::ProviderRuns;
use betterlite_core::api::assistant_run_sdk::service::{AssistantRunSdk, RunOutcome};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::thread_runs::stream::RunEventHandler;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::{Deserialize, Serialize};
use serde_with::rust::double_option;

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;
use super::vector_query_embedding::outbound_client;

mod projection;

use projection::{RecordingHandler, failure_outcome, run_outcome, stream_outcome};

type Sdk<'data> = AssistantRunSdk<'data, ProviderRuns<ProxiedClient>>;

// the four public run wrappers
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Run,
    RunStream,
    Arun,
    ArunStream,
}

// run_thread's arguments; stream distinguishes an absent kwarg from a null one
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub mode: RunMode,
    pub provider: String,
    pub thread_id: String,
    pub assistant_id: String,
    pub additional_instructions: Option<String>,
    pub instructions: Option<String>,
    pub metadata: Option<JsonValue>,
    pub model: Option<String>,
    #[serde(default, with = "double_option")]
    pub stream: Option<Option<RunFlag>>,
    pub tools: Option<JsonValue>,
    #[serde(default)]
    pub event_handler: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arun_thread: Option<RunFlag>,
    // kept untyped so the Python child receives the kwargs as written; the native call parses them
    #[serde(default)]
    pub params: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub run: Option<RunView>,
    pub stream: Option<StreamView>,
    pub failure: Option<Failure>,
    pub response: Option<ResponseView>,
}

// the fields LiteLLM's Run exposes, plus what requires_action and last_error retained
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunView {
    pub id: String,
    pub status: String,
    pub tool_calls: Option<usize>,
    pub last_error: Option<LastError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LastError {
    pub code: Option<String>,
    pub message: Option<String>,
}

// a consumed manager: the ordered event names and the supplied handler's callbacks
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StreamView {
    pub events: Vec<String>,
    pub handler: Option<HandlerView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HandlerView {
    pub events: Vec<String>,
    pub ended: bool,
}

// the httpx response litellm's own BadRequestError carries for an unsupported provider
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseView {
    pub content: String,
    pub method: String,
}

impl CallKind for Call {
    async fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<SdkResult, SdkHttpError> {
        execute(self, api_base, env).await
    }
}

async fn execute(
    input: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let loaded = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    // B2487 resolves the settings from the environment; the shared conversion copies them across
    let settings = ResourceSdkSettings::from(&loaded.sections.resource_sdk_settings);
    let azure = AzureSettings::default();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let operations = ProviderRuns {
        transport: Arc::new(outbound_client(env)?),
    };
    let sdk = Sdk {
        operations,
        settings: &settings,
        azure: &azure,
        environment: env,
        sources: &NoSources,
        redactor: &redactor,
    };
    let recorder = input.event_handler.then(RecordingHandler::default);
    let handler: Option<Arc<dyn RunEventHandler>> = recorder
        .as_ref()
        .map(|recording| Arc::new(recording.clone()) as Arc<dyn RunEventHandler>);
    let call = thread_call(input, api_base, handler)?;
    let value = run_mode(input.mode, &sdk, call, recorder.as_ref()).await;
    Ok(SdkResult::AssistantRun {
        value: Box::new(value),
    })
}

async fn run_mode(
    mode: RunMode,
    sdk: &Sdk<'_>,
    call: RunThreadCall,
    recorder: Option<&RecordingHandler>,
) -> Outcome {
    match mode {
        RunMode::RunStream => match sdk.run_thread_stream(call) {
            Ok(stream) => stream_outcome(stream, recorder).await,
            Err(error) => failure_outcome(&error),
        },
        RunMode::ArunStream => match sdk.arun_thread_stream(call) {
            Ok(stream) => stream_outcome(stream, recorder).await,
            Err(error) => failure_outcome(&error),
        },
        RunMode::Run => match sdk.run_thread(call).await {
            Ok(RunOutcome::Polled(reply)) => run_outcome(reply.as_ref()),
            Ok(RunOutcome::Streaming(stream)) => stream_outcome(stream, recorder).await,
            Err(error) => failure_outcome(&error),
        },
        RunMode::Arun => match sdk.arun_thread(call).await {
            Ok(RunOutcome::Polled(reply)) => run_outcome(reply.as_ref()),
            Ok(RunOutcome::Streaming(stream)) => stream_outcome(stream, recorder).await,
            Err(error) => failure_outcome(&error),
        },
    }
}

fn thread_call(
    input: &Call,
    api_base: &str,
    handler: Option<Arc<dyn RunEventHandler>>,
) -> Result<RunThreadCall, SdkHttpError> {
    let params = serde_json::from_value::<RunParams>(JsonValue::Object(
        input.params.clone().into_iter().collect(),
    ))
    .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    Ok(RunThreadCall {
        provider: input.provider.clone(),
        thread_id: input.thread_id.clone(),
        assistant_id: input.assistant_id.clone(),
        additional_instructions: input.additional_instructions.clone(),
        instructions: input.instructions.clone(),
        metadata: input.metadata.clone(),
        model: input.model.clone(),
        stream: input.stream.clone(),
        tools: input.tools.clone(),
        client: None,
        event_handler: handler,
        arun_thread: input.arun_thread.clone(),
        params: RunParams {
            api_base: Some(api_base.to_owned()),
            ..params
        },
    })
}
