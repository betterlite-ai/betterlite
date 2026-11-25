use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::pass_through_logging::interrupted_stream_usage::testing::{
    ChatSpec, clear_placeholder_cost, recover_with_fixed_count, stream_was_interrupted,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    tails: Vec<TailCall>,
    recovers: Vec<RecoverCall>,
    clears: Vec<ChatSpec>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TailCall {
    chunks: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct RecoverCall {
    model: String,
    count: i64,
    content: Option<String>,
    chunks: Vec<String>,
    #[serde(flatten)]
    counts: Counts,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Counts {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    has_details: bool,
    text_tokens: Option<i64>,
}

#[derive(Serialize)]
struct RecoveredAnswer {
    changed: bool,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    text_tokens: Option<i64>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let tails: Vec<Value> = input
            .tails
            .iter()
            .map(tail_answer)
            .collect::<Result<_, _>>()?;
        let recovers: Vec<Value> = input
            .recovers
            .iter()
            .map(recover_answer)
            .collect::<Result<_, _>>()?;
        let clears: Vec<Value> = input.clears.iter().map(clear_answer).collect();
        Ok(json!({
            "tails": tails,
            "recovers": recovers,
            "clears": clears,
        }))
    }
}

fn tail_answer(call: &TailCall) -> Result<Value, Box<HelperError>> {
    let chunks = chunks_of(&call.chunks)?;
    let interrupted = stream_was_interrupted(&chunks).or_value_error()?;
    Ok(json!(interrupted))
}

fn recover_answer(call: &RecoverCall) -> Result<Value, Box<HelperError>> {
    let spec = ChatSpec {
        content: call.content.clone(),
        prompt_tokens: call.counts.prompt_tokens,
        completion_tokens: call.counts.completion_tokens,
        total_tokens: call.counts.total_tokens,
        has_details: call.counts.has_details,
        text_tokens: call.counts.text_tokens,
        cost: None,
        hidden_response_cost: None,
    };
    let outcome =
        recover_with_fixed_count(&spec, &chunks_of(&call.chunks)?, &call.model, call.count)
            .or_value_error()?;
    let answer = RecoveredAnswer {
        changed: outcome.changed,
        prompt_tokens: outcome.usage.prompt_tokens,
        completion_tokens: outcome.usage.completion_tokens,
        total_tokens: outcome.usage.total_tokens,
        text_tokens: outcome.usage.text_tokens,
    };
    serde_json::to_value(answer).or_value_error()
}

fn clear_answer(spec: &ChatSpec) -> Value {
    let outcome = clear_placeholder_cost(spec);
    json!({
        "cost_cleared": outcome.cost_cleared,
        "hidden_cost_cleared": outcome.hidden_cost_cleared,
    })
}

fn chunks_of(encoded: &[String]) -> Result<Vec<Vec<u8>>, Box<HelperError>> {
    encoded
        .iter()
        .map(|chunk| hex::decode(chunk).or_value_error())
        .collect()
}
