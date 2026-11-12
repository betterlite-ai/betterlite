use crate::helpers::{Helper, HelperError, HelperResult};
use serde::Deserialize;
use serde_json::Value;

mod oobabooga;
mod servers;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ProviderInfo {
        calls: Vec<servers::ProviderCall>,
    },
    ChatParams {
        calls: Vec<servers::ParamsCall>,
    },
    DockerRequest {
        calls: Vec<servers::DockerCall>,
    },
    LmStudioEmbed,
    Xinference {
        calls: Vec<servers::XinferenceCall>,
    },
    LemonadeCost {
        calls: Vec<servers::CostCall>,
    },
    OobaboogaRequest {
        calls: Vec<oobabooga::RequestCall>,
    },
    OobaboogaEmbedding {
        calls: Vec<oobabooga::EmbeddingCall>,
    },
    OobaboogaReply {
        calls: Vec<oobabooga::ReplyCall>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ProviderInfo { calls } => Ok(Value::Array(
                calls.iter().map(servers::provider_value).collect(),
            )),
            Input::ChatParams { calls } => Ok(Value::Array(
                calls.iter().map(servers::params_value).collect(),
            )),
            Input::DockerRequest { calls } => answers(calls, servers::docker_value),
            Input::LmStudioEmbed => Ok(servers::embed_value()),
            Input::Xinference { calls } => Ok(Value::Array(
                calls.iter().map(servers::xinference_value).collect(),
            )),
            Input::LemonadeCost { calls } => Ok(Value::Array(
                calls.iter().map(servers::cost_value).collect(),
            )),
            Input::OobaboogaRequest { calls } => answers(calls, |call| {
                oobabooga::answer(&oobabooga::Call::Request(call))
            }),
            Input::OobaboogaEmbedding { calls } => answers(calls, |call| {
                oobabooga::answer(&oobabooga::Call::Embedding(call))
            }),
            Input::OobaboogaReply { calls } => answers(calls, |call| {
                oobabooga::answer(&oobabooga::Call::Reply(call))
            }),
        }
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn answers<Call, Make>(calls: &[Call], make: Make) -> HelperResult
where
    Make: Fn(&Call) -> Result<Value, Box<HelperError>>,
{
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        values.push(make(call)?);
    }
    Ok(Value::Array(values))
}
