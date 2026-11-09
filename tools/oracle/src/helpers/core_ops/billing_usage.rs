use std::fmt::Display;

use betterlite_core::cost::usage::ResponseKind;
use serde::Deserialize;
use serde_json::{Value, json};

mod parse;
mod probes;
mod projection;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    #[serde(default)]
    response: Option<ResponseInput>,
    #[serde(default)]
    variant: Option<UsageSpec>,
    #[serde(default)]
    kind: Option<KindInput>,
    #[serde(default)]
    usage: Option<Value>,
    #[serde(default)]
    hidden_params: Option<Value>,
    #[serde(default)]
    requested_tier: Option<String>,
    #[serde(default)]
    served_tier: Option<String>,
    #[serde(default, rename = "call_type")]
    explicit_type: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    GetUsageObject,
    IsKnownUsage,
    InferCallType,
    TranscriptionHasTokenDetails,
    WithoutProviderStatedCost,
    HiddenResponseCost,
    ServedServiceTier,
    ResolveBillableServiceTier,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum KindInput {
    Chat,
    ChatStream,
    Embedding,
    Transcription,
    Speech,
    Rerank,
    Image,
    TextCompletion,
    SendMessage,
    DecisionsIr,
    DecisionsOpenAi,
    Responses,
    Other,
}

impl KindInput {
    const fn response_kind(self) -> ResponseKind {
        match self {
            Self::Chat => ResponseKind::Chat,
            Self::ChatStream => ResponseKind::ChatStream,
            Self::Embedding => ResponseKind::Embedding,
            Self::Transcription => ResponseKind::Transcription,
            Self::Speech => ResponseKind::Speech,
            Self::Rerank => ResponseKind::Rerank,
            Self::Image => ResponseKind::Image,
            Self::TextCompletion => ResponseKind::TextCompletion,
            Self::SendMessage => ResponseKind::SendMessage,
            Self::DecisionsIr => ResponseKind::DecisionsIr,
            Self::DecisionsOpenAi => ResponseKind::DecisionsOpenAi,
            Self::Responses => ResponseKind::Responses,
            Self::Other => ResponseKind::Other,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseInput {
    kind: KindInput,
    #[serde(default)]
    usage: Option<UsageSpec>,
    #[serde(default)]
    service_tier: Option<String>,
    #[serde(default)]
    hidden_params: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageSpec {
    kind: UsageKindInput,
    #[serde(default)]
    value: Option<Value>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UsageKindInput {
    Absent,
    Null,
    Chat,
    Responses,
    Transcription,
    DecisionsIr,
    DecisionsOpenAi,
    Interactions,
    Mapping,
    TypedRecord,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcomes = input.calls.iter().map(run_call).collect::<Vec<_>>();
        Ok(Value::Array(outcomes))
    }
}

fn run_call(call: &Call) -> Value {
    match execute(call) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => json!({"ok": false, "error": {"class": error.class}}),
    }
}

fn execute(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    match call.function {
        Function::GetUsageObject => probes::get_usage(call),
        Function::IsKnownUsage => probes::known_usage(call),
        Function::InferCallType => probes::infer(call),
        Function::TranscriptionHasTokenDetails => probes::transcription_details(call),
        Function::WithoutProviderStatedCost => probes::cleared_usage(call),
        Function::HiddenResponseCost => probes::hidden_cost(call),
        Function::ServedServiceTier => probes::served_tier(call),
        Function::ResolveBillableServiceTier => Ok(probes::resolve_tier(call)),
    }
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
