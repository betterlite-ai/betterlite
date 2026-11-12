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
    LmStudioEmbed {
        optional_params: betterlite_openai::openai_base_config::Object,
    },
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
            Input::DockerRequest { calls } => Ok(Value::Array(
                calls
                    .iter()
                    .map(servers::docker_value)
                    .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            )),
            Input::LmStudioEmbed { optional_params } => Ok(servers::embed_value(optional_params)),
            Input::Xinference { calls } => Ok(Value::Array(
                calls.iter().map(servers::xinference_value).collect(),
            )),
            Input::LemonadeCost { calls } => Ok(Value::Array(
                calls.iter().map(servers::cost_value).collect(),
            )),
            Input::OobaboogaRequest { calls } => Ok(Value::Array(
                calls
                    .iter()
                    .map(|call| {
                        oobabooga::finish(oobabooga::record(&oobabooga::Call::Request(call)))
                    })
                    .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            )),
            Input::OobaboogaEmbedding { calls } => Ok(Value::Array(
                calls
                    .iter()
                    .map(|call| {
                        oobabooga::finish(oobabooga::record(&oobabooga::Call::Embedding(call)))
                    })
                    .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            )),
            Input::OobaboogaReply { calls } => Ok(Value::Array(
                calls
                    .iter()
                    .map(|call| oobabooga::finish(oobabooga::record(&oobabooga::Call::Reply(call))))
                    .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            )),
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
