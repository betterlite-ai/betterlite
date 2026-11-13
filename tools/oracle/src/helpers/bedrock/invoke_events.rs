use std::collections::BTreeMap;

use betterlite_bedrock::invoke_events::fake::{FakeResponse, FakeStream};
use betterlite_bedrock::invoke_events::{
    DecoderClock, DecoderOptions, InvokeStreamDecoder, anthropic_decoder, base_decoder,
    bedrock_event_stream_decoder, deepseek_decoder, openai_compatible_decoder,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use bytes::Bytes;
use futures_util::StreamExt as _;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;
use views::{chunk_view, generic_view};

use crate::helpers::{HelperError, HelperResult};

mod tests;
mod views;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    framed: Vec<FramedScenario>,
    #[serde(default)]
    fake: Vec<FakeScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FramedScenario {
    construction: Construction,
    #[serde(default)]
    provider: Option<String>,
    sync: bool,
    model: String,
    #[serde(default)]
    json_mode: Option<bool>,
    chunks: Vec<String>,
    #[serde(default)]
    headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Construction {
    Legacy,
    Anthropic,
    DeepseekR1,
    Moonshot,
    Base,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FakeScenario {
    response: FakeResponse,
    #[serde(default)]
    json_mode: Option<bool>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Step {
    Next,
    Anext,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        let framed = input
            .framed
            .iter()
            .map(|scenario| run_framed(scenario, &runtime))
            .collect::<Result<Vec<_>, _>>()?;
        let fake = input
            .fake
            .iter()
            .map(|scenario| run_fake(scenario, &runtime))
            .collect::<Vec<_>>();
        Ok(json!({ "framed": framed, "fake": fake }))
    }
}
fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvokeEvents".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
fn current_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}
fn decoder(scenario: &FramedScenario) -> InvokeStreamDecoder {
    let clock = DecoderClock {
        next_id: Uuid::new_v4,
        now: current_seconds,
    };
    let options = DecoderOptions {
        model: &scenario.model,
        json_mode: scenario.json_mode,
        sync_stream: scenario.sync,
        clock,
    };
    match scenario.construction {
        Construction::Legacy => bedrock_event_stream_decoder(scenario.provider.as_deref(), &options),
        Construction::Anthropic => anthropic_decoder(&options),
        Construction::DeepseekR1 => deepseek_decoder(&options),
        Construction::Moonshot => openai_compatible_decoder(&options),
        Construction::Base => base_decoder(&options),
    }
}
fn run_framed(
    scenario: &FramedScenario,
    runtime: &tokio::runtime::Runtime,
) -> Result<Value, Box<HelperError>> {
    let chunks = scenario
        .chunks
        .iter()
        .map(|chunk| hex::decode(chunk).map(Bytes::from))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| failure(error.to_string()))?;
    let mut decoder = decoder(scenario);
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failure(error.to_string()))?;
    let outcomes = if scenario.sync {
        decoder
            .iter_bytes(chunks.into_iter(), scenario.headers.clone(), &redactor)
            .collect::<Vec<_>>()
    } else {
        runtime.block_on(async {
            let mut stream = decoder.aiter_bytes(
                futures_util::stream::iter(chunks),
                scenario.headers.clone(),
                &redactor,
            );
            let mut outcomes = Vec::new();
            while let Some(outcome) = stream.next().await {
                outcomes.push(outcome);
            }
            outcomes
        })
    };
    let mut views = Vec::new();
    let mut failure_view = Value::Null;
    for outcome in outcomes {
        match outcome {
            Ok(chunk) => views.push(chunk_view(chunk)),
            Err(error) => {
                failure_view = json!({ "status": error.status() });
                break;
            }
        }
    }
    Ok(json!({ "chunks": views, "failure": failure_view }))
}

fn run_fake(scenario: &FakeScenario, runtime: &tokio::runtime::Runtime) -> Value {
    let mut stream = FakeStream {
        response: scenario.response.clone(),
        json_mode: scenario.json_mode,
        is_done: false,
    };
    let mut steps = Vec::new();
    for step in &scenario.steps {
        let outcome = match step {
            Step::Next => Iterator::next(&mut stream),
            Step::Anext => runtime.block_on(StreamExt::next(&mut stream)),
        };
        steps.push(match outcome {
            Some(Ok(chunk)) => json!({ "ok": generic_view(chunk) }),
            Some(Err(error)) => json!({ "failure": { "status": error.status() } }),
            None => json!({ "done": true }),
        });
    }
    json!({ "steps": steps })
}
