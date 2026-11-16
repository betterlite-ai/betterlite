use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::realtime_contracts::config::BaseRealtimeConfig as _;
use betterlite_core::api::realtime_contracts::error::WebSocketError;
use betterlite_core::api::realtime_contracts::events::RealtimeResponseTransformInput;
use betterlite_core::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use betterlite_core::api::vertex_chirp::config::VertexChirpRealtimeConfig;
use betterlite_core::api::vertex_chirp::error::ChirpAdapterError;
use betterlite_core::api::vertex_chirp::factory::ChirpBackendFactory;
use betterlite_core::api::vertex_chirp::ids::UuidChirpIds;
use betterlite_core::api::vertex_chirp::settings::ChirpSettings;
use betterlite_core::api::vertex_chirp::target::{
    AccessTokenError, AccessTokenFuture, AccessTokenResolver, SpeechStreamingTarget,
};
use betterlite_models::catalog::BundledCatalog;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use inputs::{FrameInput, Function, ResponsesCall, UrlCase};
use outputs::{
    CommandOutput, CommandsOutput, ResponseErrorOutput, ResponseOutput, ResponsesOutput,
    StepOutput, UrlOutput, UrlsOutput,
};

mod inputs;
mod outputs;
mod tests;

type ChirpConfig = VertexChirpRealtimeConfig<UnusedFactory>;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Function;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Function::Commands {
                project,
                location,
                model,
                session_id,
                steps,
            } => {
                let config = adapter(project, location.as_deref()).or_config_error()?;
                commands(&config, model, session_id.as_deref(), steps)
            }
            Function::Urls {
                project,
                location,
                cases,
            } => {
                let config = adapter(project, location.as_deref()).or_config_error()?;
                urls(&config, cases)
            }
            Function::Responses(call) => {
                let config = adapter(&call.project, call.location.as_deref()).or_config_error()?;
                responses(&config, call)
            }
        }
    }
}

fn adapter(project: &str, location: Option<&str>) -> Result<ChirpConfig, ChirpAdapterError> {
    VertexChirpRealtimeConfig::new(
        ChirpSettings {
            project: project.to_owned(),
            location: location.map(str::to_owned),
            catalog: BundledCatalog::default().get(),
            limits: ApiLimits::default(),
            ids: Box::new(UuidChirpIds::default()),
        },
        Arc::new(UnusedToken),
        UnusedFactory,
    )
}

fn commands(
    config: &ChirpConfig,
    model: &str,
    session_id: Option<&str>,
    steps: &[String],
) -> HelperResult {
    let created = session_id.and_then(|logging_session_id| {
        config.session_created_event(model, logging_session_id, None)
    });
    let outputs = steps
        .iter()
        .map(|step| step_output(config, model, step))
        .collect();
    let usage = config.session_close_usage(model);
    let repeated_usage = config.session_close_usage(model);
    let output = CommandsOutput {
        created,
        steps: outputs,
        usage,
        repeated_usage,
    };
    serde_json::to_value(&output).or_value_error()
}

fn step_output(config: &ChirpConfig, model: &str, step: &str) -> StepOutput {
    let commands = config.transform_realtime_request(step, model, None);
    StepOutput {
        error: commands.is_err(),
        commands: commands.iter().flatten().map(command_output).collect(),
    }
}

fn command_output(command: &RealtimeMessage) -> CommandOutput {
    match command {
        RealtimeMessage::Text(value) => CommandOutput::Text {
            value: value.clone(),
        },
        RealtimeMessage::Binary(bytes) => CommandOutput::Audio {
            base64: STANDARD.encode(bytes),
        },
    }
}

fn urls(config: &ChirpConfig, cases: &[UrlCase]) -> HelperResult {
    let urls: Vec<UrlOutput> = cases
        .iter()
        .map(|case| {
            let url = config
                .complete_url(case.api_base.as_deref(), &case.model)
                .ok();
            UrlOutput {
                error: url.is_none(),
                url,
            }
        })
        .collect();
    serde_json::to_value(&UrlsOutput { urls }).or_value_error()
}

fn responses(config: &ChirpConfig, call: &ResponsesCall) -> HelperResult {
    let model = call.model.as_str();
    let created = config.session_created_event(model, &call.session_id, None);
    if let Some(request) = call.configure.as_deref() {
        config
            .transform_realtime_request(request, model, None)
            .or_value_error()?;
    }
    let responses = call
        .frames
        .iter()
        .map(|frame| response_output(config, model, &call.state, frame))
        .collect::<Result<Vec<_>, _>>()?;
    let usage = config.session_close_usage(model);
    let repeated_usage = config.session_close_usage(model);
    let output = ResponsesOutput {
        created,
        responses,
        usage,
        repeated_usage,
    };
    serde_json::to_value(&output).or_value_error()
}

fn response_output(
    config: &ChirpConfig,
    model: &str,
    state: &RealtimeResponseTransformInput,
    frame: &FrameInput,
) -> Result<ResponseOutput, Box<HelperError>> {
    let message = frame_message(frame)?;
    Ok(
        match config.transform_realtime_response(&message, model, state) {
            Ok(result) => ResponseOutput {
                result: Some(result),
                error: None,
            },
            Err(ChirpAdapterError::Event { .. }) => ResponseOutput {
                result: None,
                error: Some(ResponseErrorOutput::Invalid),
            },
            Err(_) => ResponseOutput {
                result: None,
                error: Some(ResponseErrorOutput::Protocol),
            },
        },
    )
}

fn frame_message(frame: &FrameInput) -> Result<RealtimeMessage, Box<HelperError>> {
    match frame {
        FrameInput::Text { value } => Ok(RealtimeMessage::Text(value.clone())),
        FrameInput::Bytes { base64 } => STANDARD
            .decode(base64)
            .map(RealtimeMessage::Binary)
            .or_value_error(),
    }
}

struct UnusedFactory;

impl ChirpBackendFactory for UnusedFactory {
    type Backend = UnusedBackend;

    fn open(&self, _target: SpeechStreamingTarget) -> Self::Backend {
        UnusedBackend
    }
}

struct UnusedBackend;

#[async_trait]
impl RealtimeBackend for UnusedBackend {
    async fn send(&mut self, _message: RealtimeMessage) -> Result<(), WebSocketError> {
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Err(WebSocketError {
            message: "the unused backend never receives".to_owned(),
        })
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        Ok(())
    }
}

struct UnusedToken;

impl AccessTokenResolver for UnusedToken {
    fn resolve_access_token(&self) -> AccessTokenFuture<'_> {
        Box::pin(async { Err(AccessTokenError) })
    }
}
