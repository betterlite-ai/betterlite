use std::fmt::Display;

use betterlite_core::cost::usage::ResponseKind;
use betterlite_core::cost::usage::provider_cost::{
    hidden_response_cost, without_provider_stated_cost,
};
use betterlite_core::cost::usage::tiers::resolve_billable_service_tier;
use serde::Deserialize;
use serde_json::{Value, json};

use model::{decimal_value, optional_mapping, optional_text, optional_usage, usage_projection};

mod model;
mod probes;

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
    usage: Option<Value>,
    #[serde(default)]
    hidden_params: Option<Value>,
    #[serde(default)]
    requested_tier: Option<String>,
    #[serde(default)]
    served_tier: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    GetUsageObject,
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
        Function::WithoutProviderStatedCost => {
            let Some(usage) = optional_usage(call)? else {
                return Ok(Value::Null);
            };
            let cleared = without_provider_stated_cost(&usage);
            Ok(usage_projection(Some(cleared.as_ref())))
        }
        Function::HiddenResponseCost => {
            let hidden = optional_mapping(call.hidden_params.as_ref())?;
            let cost = hidden_response_cost(hidden.as_ref()).map_err(failure)?;
            Ok(cost.map_or(Value::Null, |amount| decimal_value(&amount)))
        }
        Function::ServedServiceTier => probes::served_tier(call),
        Function::ResolveBillableServiceTier => Ok(optional_text(
            resolve_billable_service_tier(
                call.requested_tier.as_deref(),
                call.served_tier.as_deref(),
            )
            .as_deref(),
        )),
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
