use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::guardrails::streaming_rewrite::shapes::{RequestData, StreamItem};
use betterlite_core::guardrails::streaming_rewrite::step::{StreamingStep, run_streaming_step};
use betterlite_core::guardrails::streaming_rewrite::traits::UserApiKey;
use betterlite_guardrails::guardrail_message_scope::stream_chunk_events::stream_item_value;
use serde::Serialize;
use thiserror::Error;

mod fakes;
mod input;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use fakes::{Scripted, StepError, Translation};
use input::Input;
use std::io;
use tokio::runtime::Builder;

fn record_applied(request_data: &mut RequestData, guardrail_name: &str) {
    let applied = request_data
        .entry("applied_guardrails".to_owned())
        .or_insert_with(|| GuardrailValue::Array(Vec::new()));
    if let GuardrailValue::Array(names) = applied {
        names.push(GuardrailValue::Text(guardrail_name.to_owned()));
    }
}

#[derive(Debug, Error)]
enum RunError {
    #[error(transparent)]
    Step(#[from] StepError),
    #[error(transparent)]
    Runtime(#[from] io::Error),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
}

// LiteLLM's executor turns a step exception into outcome "error", so only a passing step reaches the output
#[derive(Serialize)]
struct Output {
    outcome: &'static str,
    chunks: Vec<GuardrailValue>,
    applied_guardrails: GuardrailValue,
    delivered_tool_calls: Option<GuardrailValue>,
}

async fn drive<const DELIVERS: bool>(input: &Input) -> Result<Output, StepError> {
    let mut guardrail = Scripted {
        script: &input.guardrail,
    };
    let translation = Translation::<DELIVERS> {
        script: &input.translation,
    };
    let mut items: Vec<StreamItem> = input
        .chunks
        .iter()
        .cloned()
        .map(|chunk| StreamItem::Json(GuardrailValue::Text(chunk)))
        .collect();
    let mut hook_input = RequestData::new();
    let user_api_key_dict = UserApiKey::new();
    let step = StreamingStep {
        guardrail: &input.guardrail.name,
        streaming_chunks: &mut items,
        hook_input: &mut hook_input,
        user_api_key_dict: &user_api_key_dict,
        logging_obj: None::<&mut ()>,
    };
    run_streaming_step(&mut guardrail, &translation, step, record_applied).await?;
    Ok(Output {
        outcome: "pass",
        chunks: items.iter().map(stream_item_value).collect(),
        applied_guardrails: hook_input
            .remove("applied_guardrails")
            .unwrap_or_else(|| GuardrailValue::Array(Vec::new())),
        delivered_tool_calls: hook_input.remove("delivered_tool_calls"),
    })
}

fn execute(input: &Input) -> Result<serde_json::Value, RunError> {
    let runtime = Builder::new_current_thread().build()?;
    let output = if input.translation.delivers {
        runtime.block_on(drive::<true>(input))?
    } else {
        runtime.block_on(drive::<false>(input))?
    };
    Ok(serde_json::to_value(output)?)
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        execute(input).map_err(|error| {
            Box::new(HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
