use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::rubrik::block_decision::ResponseBlockError;
use betterlite_logging::rubrik::tool_calls::ToolCallSource;
use betterlite_logging::rubrik_delivery::config::RubrikInit;
use betterlite_logging::rubrik_delivery::error::{RubrikError, Service};
use betterlite_logging::rubrik_delivery::logger::RubrikLogger;
use betterlite_logging::rubrik_delivery::transport::TransportFailure;
use betterlite_logging::rubrik_moderation::apply::apply_guardrail;
use betterlite_logging::rubrik_moderation::context::{
    LoggingObject, RequestData, stash_block_context,
};
use betterlite_logging::rubrik_moderation::error::{ModerationError, ModifyResponseException};
use betterlite_logging::rubrik_moderation::guarded::guarded;
use betterlite_logging::rubrik_moderation::inputs::{GuardrailInputs, ModerationInputType};
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};

mod decode;
mod scripted;

use decode::{blocked_result, failure, returned_result, stashed_result};
use scripted::{LogCapture, Reply, ScriptedClock, ScriptedTransport};

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    environment: BTreeMap<String, String>,
    init: Init,
    clock: Vec<f64>,
    replies: Vec<Reply>,
    logging_obj: Option<Value>,
    request_data: Option<Value>,
    actions: Vec<Action>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Init {
    api_key: Option<String>,
    api_base: Option<String>,
    guardrail_name: Option<String>,
    timeout_seconds: Option<f64>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Fault {
    Blocked,
    Malformed,
    Transient,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct InputsInput {
    texts: Vec<String>,
    tool_calls: Vec<Json>,
    structured_messages: Vec<Json>,
    model: Option<String>,
    tools: Option<Json>,
}

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    ApplyGuardrail {
        input_type: String,
        #[serde(default)]
        inputs: InputsInput,
    },
    Guarded {
        fault: Fault,
        #[serde(default)]
        inputs: InputsInput,
    },
    StashBlockContext,
}

impl InputsInput {
    fn to_guardrail_inputs(&self) -> GuardrailInputs {
        GuardrailInputs {
            texts: self.texts.clone(),
            tool_calls: self
                .tool_calls
                .iter()
                .cloned()
                .map(ToolCallSource::Raw)
                .collect(),
            structured_messages: self.structured_messages.clone(),
            model: self.model.clone(),
            tools: self.tools.clone(),
        }
    }
}

fn moderation_input_type(text: &str) -> ModerationInputType {
    match text {
        "request" => ModerationInputType::Request,
        "response" => ModerationInputType::Response,
        _ => ModerationInputType::Unrecognized,
    }
}

fn seconds(value: Option<f64>) -> Result<Option<Duration>, Box<HelperError>> {
    value
        .map(|seconds| {
            Duration::try_from_secs_f64(seconds)
                .map_err(|error| failure(format!("init.timeout_seconds: {error}")))
        })
        .transpose()
}

fn rubrik_init(init: &Init) -> Result<RubrikInit, Box<HelperError>> {
    Ok(RubrikInit {
        api_key: init.api_key.clone().map(SecretString::from),
        api_base: init.api_base.clone(),
        event_hook: None,
        default_on: None,
        guardrail_name: init.guardrail_name.clone(),
        timeout: seconds(init.timeout_seconds)?,
        request_timeout: None,
    })
}

fn scripted_clock(values: &[f64]) -> Result<ScriptedClock, Box<HelperError>> {
    let durations = values
        .iter()
        .map(|seconds| {
            Duration::try_from_secs_f64(*seconds)
                .map_err(|error| failure(format!("clock entry: {error}")))
        })
        .collect::<Result<Vec<Duration>, Box<HelperError>>>()?;
    Ok(ScriptedClock::new(&durations))
}

const fn transient_failure() -> RubrikError {
    RubrikError::Transport {
        service: Service::ResponseModeration,
        source: TransportFailure::Request {
            source: TransportError::Rejected(HttpError::Timeout),
        },
    }
}

async fn perform_apply(
    logger: &RubrikLogger,
    input_type: &str,
    inputs: &InputsInput,
    request_data: &mut RequestData,
    logging_obj: Option<&Arc<LoggingObject>>,
) -> Result<Value, Box<HelperError>> {
    let outcome = apply_guardrail(
        logger,
        inputs.to_guardrail_inputs(),
        request_data,
        moderation_input_type(input_type),
        logging_obj,
    )
    .await;
    Ok(match outcome {
        Ok(returned) => returned_result(&returned, request_data, logging_obj),
        Err(exception) => blocked_result(&exception, request_data, logging_obj),
    })
}

async fn perform_guarded(
    logger: &RubrikLogger,
    fault: &Fault,
    inputs: &InputsInput,
    request_data: &RequestData,
    logging_obj: Option<&Arc<LoggingObject>>,
) -> Result<Value, Box<HelperError>> {
    let block_data = request_data.clone();
    let guardrail_name = logger.settings.guardrail_name.clone();
    let operation = async move {
        match fault {
            Fault::Blocked => Err::<GuardrailInputs, ModerationError>(ModerationError::Blocked(
                Box::new(ModifyResponseException {
                    message: "synthetic block".to_owned(),
                    model: "gpt-4o".to_owned(),
                    request_data: Box::new(block_data),
                    guardrail_name,
                }),
            )),
            Fault::Malformed => Err(ModerationError::Malformed(
                ResponseBlockError::MalformedResponse,
            )),
            Fault::Transient => Err(ModerationError::Rubrik(transient_failure())),
        }
    };
    let outcome = guarded(
        operation,
        inputs.to_guardrail_inputs(),
        "Response moderation",
    )
    .await;
    Ok(match outcome {
        Ok(returned) => returned_result(&returned, request_data, logging_obj),
        Err(exception) => blocked_result(&exception, request_data, logging_obj),
    })
}

async fn perform(
    logger: &RubrikLogger,
    action: &Action,
    request_data: &mut RequestData,
    logging_obj: Option<&Arc<LoggingObject>>,
) -> Result<Value, Box<HelperError>> {
    match action {
        Action::ApplyGuardrail { input_type, inputs } => {
            perform_apply(logger, input_type, inputs, request_data, logging_obj).await
        }
        Action::Guarded { fault, inputs } => {
            perform_guarded(logger, fault, inputs, request_data, logging_obj).await
        }
        Action::StashBlockContext => {
            stash_block_context(logging_obj, request_data);
            Ok(stashed_result(request_data, logging_obj))
        }
    }
}

async fn run_actions(
    logger: &RubrikLogger,
    actions: &[Action],
    request_data: &mut RequestData,
    logging_obj: Option<&Arc<LoggingObject>>,
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut results = Vec::with_capacity(actions.len());
    for action in actions {
        results.push(perform(logger, action, request_data, logging_obj).await?);
    }
    Ok(results)
}

fn report(transport: &ScriptedTransport, logs: &LogCapture, results: &[Value]) -> Value {
    json!({
        "results": results,
        "calls": transport.calls(),
        "logs": logs.report(),
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let clock = Arc::new(scripted_clock(&input.clock)?);
        let transport = Arc::new(ScriptedTransport::new(&input.replies));
        let init = rubrik_init(&input.init)?;
        let environment = input.environment.clone();
        let logging_obj = decode::logging_object(input.logging_obj.as_ref())?;
        let mut request_data = decode::request_data(input.request_data.as_ref())?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| failure(format!("the helper runtime failed: {error}")))?;
        let logs = LogCapture::default();
        let subscriber = tracing_subscriber::registry().with(logs.clone());
        tracing::subscriber::with_default(subscriber, || {
            let logger = Arc::new(
                RubrikLogger::new(
                    &init,
                    &environment,
                    Arc::<ScriptedTransport>::clone(&transport),
                    clock,
                )
                .map_err(|error| failure(format!("the Rubrik logger failed: {error}")))?,
            );
            let results = runtime.block_on(run_actions(
                &logger,
                &input.actions,
                &mut request_data,
                logging_obj.as_ref(),
            ))?;
            Ok(report(&transport, &logs, &results))
        })
    }
}
