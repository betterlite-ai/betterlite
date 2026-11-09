use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::rubrik::tool_calls::ToolCallSource;
use betterlite_logging::rubrik_delivery::config::RubrikInit;
use betterlite_logging::rubrik_delivery::logger::RubrikLogger;
use betterlite_logging::rubrik_moderation::apply::apply_guardrail;
use betterlite_logging::rubrik_moderation::inputs::{GuardrailInputs, ModerationInputType};
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};

mod decode;
mod scripted;

use decode::{failure, outcome_result};
use scripted::{LogCapture, Reply, ScriptedClock, ScriptedTransport};

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Case {
    environment: BTreeMap<String, String>,
    init: Init,
    clock: Vec<f64>,
    replies: Vec<Reply>,
    logging_obj: Option<Value>,
    request_data: Option<Value>,
    input_type: String,
    inputs: InputsInput,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Init {
    api_key: Option<String>,
    api_base: Option<String>,
    guardrail_name: Option<String>,
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

fn rubrik_init(init: &Init) -> RubrikInit {
    RubrikInit {
        api_key: init.api_key.clone().map(SecretString::from),
        api_base: init.api_base.clone(),
        event_hook: None,
        default_on: None,
        guardrail_name: init.guardrail_name.clone(),
        timeout: None,
        request_timeout: None,
    }
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

fn report(transport: &ScriptedTransport, logs: &LogCapture, result: &Value) -> Value {
    json!({
        "result": result,
        "calls": transport.calls(),
        "logs": logs.report(),
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Case;

    fn run(input: &Self::Input) -> HelperResult {
        let clock = Arc::new(scripted_clock(&input.clock)?);
        let transport = Arc::new(ScriptedTransport::new(&input.replies));
        let init = rubrik_init(&input.init);
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
                    &input.environment,
                    Arc::<ScriptedTransport>::clone(&transport),
                    clock,
                )
                .map_err(|error| failure(format!("the Rubrik logger failed: {error}")))?,
            );
            let outcome = runtime.block_on(apply_guardrail(
                &logger,
                input.inputs.to_guardrail_inputs(),
                &mut request_data,
                moderation_input_type(&input.input_type),
                logging_obj.as_ref(),
            ));
            let result = outcome_result(outcome, &request_data, logging_obj.as_ref());
            Ok(report(&transport, &logs, &result))
        })
    }
}
