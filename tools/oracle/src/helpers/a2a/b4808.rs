use std::collections::{BTreeMap, HashMap};
use std::num::NonZeroUsize;
use std::sync::PoisonError;

use betterlite_a2a::watsonx_credentials::bearer::BearerTokens;
use betterlite_a2a::watsonx_credentials::token_cache::BoundedTokenCache;
use betterlite_a2a::watsonx_run::config::{Answer, ConfigCall, Reply, WatsonxRunConfig};
use betterlite_a2a::watsonx_run::handler::RunHandler;
use betterlite_a2a::watsonx_transform::extract::WxoSendParams;
use betterlite_core::agents::json::AgentJson;
use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};
use report::{CallError, Failure, Outcome, Output, PacedEvent, helper_error, kind, normalize};
use script::{Clock, Script, Source};

mod report;
mod script;
mod tests;

pub(crate) struct Op;

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Mode {
    NonStreaming,
    Streaming,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Fail {
    Transport,
    Read,
}

#[derive(Clone, Deserialize)]
struct Step {
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    body: AgentJson,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    lines: Vec<String>,
    #[serde(default)]
    fail: Option<Fail>,
    #[serde(default)]
    repeat: bool,
}

const fn default_status() -> u16 {
    200
}

fn default_request_id() -> String {
    "request".to_owned()
}

#[derive(Deserialize)]
pub(crate) struct Input {
    mode: Mode,
    #[serde(default = "default_request_id")]
    request_id: String,
    params: WxoSendParams,
    #[serde(default)]
    kwargs: BTreeMap<String, AgentJson>,
    #[serde(default)]
    steps: Vec<Step>,
}

async fn call(
    config: &mut WatsonxRunConfig<&Script, &Script, Clock, BoundedTokenCache, Source>,
    input: &Input,
) -> Result<AgentJson, CallError> {
    let request = ConfigCall {
        request_id: &input.request_id,
        params: &input.params,
        kwargs: &input.kwargs,
    };
    let reply = match input.mode {
        Mode::NonStreaming => Reply::Message,
        Mode::Streaming => Reply::Stream,
    };
    match config.handle(&request, reply).await? {
        Answer::Message(event) => Ok(serde_json::to_value(event)?),
        Answer::Stream(steps) => {
            let paced = steps
                .iter()
                .map(|step| PacedEvent {
                    event: &step.event,
                    delay_ms: u64::try_from(step.delay_after.as_millis()).unwrap_or(u64::MAX),
                })
                .collect::<Vec<_>>();
            Ok(serde_json::to_value(paced)?)
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| helper_error("RuntimeError", error))?;
        let script = Script::new(input.steps.clone());
        let mut config = WatsonxRunConfig {
            handler: RunHandler {
                transport: &script,
                credentials: BearerTokens {
                    transport: &script,
                    clock: Clock,
                    store: BoundedTokenCache::new(NonZeroUsize::MIN),
                },
                source: Source::default(),
            },
        };
        let outcome = match runtime.block_on(call(&mut config, input)) {
            Ok(value) => Outcome::Ok(value),
            Err(error) => Outcome::Error(Failure { kind: kind(&error) }),
        };
        drop(config);
        let output = Output {
            outcome,
            requests: script
                .requests
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner),
            sleeps: script
                .pauses
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner),
        };
        let mut value = serde_json::to_value(output)
            .map_err(|error| helper_error("SerializationError", error))?;
        normalize(&mut value, &mut HashMap::new());
        Ok(value)
    }
}
