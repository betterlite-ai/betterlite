use betterlite_models::decisions::{Extra, Json};
use serde_json::Number;

use super::super::error::PredictionError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PredictCall {
    Chat,
    ChatStreaming,
    Text,
    TextStreaming,
}

#[derive(Clone, Copy)]
enum Parameter {
    Integer,
    Number,
    Texts,
    Object,
}

const CHAT: u8 = 1;
const CHAT_STREAMING: u8 = 2;
const TEXT: u8 = 4;
const TEXT_STREAMING: u8 = 8;

// The SDK kwargs each predict method accepts, with the wire name its request builder uses
const PARAMETERS: [(&str, &str, Parameter, u8); 11] = [
    (
        "max_output_tokens",
        "maxDecodeSteps",
        Parameter::Integer,
        CHAT | CHAT_STREAMING | TEXT | TEXT_STREAMING,
    ),
    (
        "temperature",
        "temperature",
        Parameter::Number,
        CHAT | CHAT_STREAMING | TEXT | TEXT_STREAMING,
    ),
    (
        "top_k",
        "topK",
        Parameter::Integer,
        CHAT | CHAT_STREAMING | TEXT | TEXT_STREAMING,
    ),
    (
        "top_p",
        "topP",
        Parameter::Number,
        CHAT | CHAT_STREAMING | TEXT | TEXT_STREAMING,
    ),
    (
        "stop_sequences",
        "stopSequences",
        Parameter::Texts,
        CHAT | CHAT_STREAMING | TEXT | TEXT_STREAMING,
    ),
    (
        "candidate_count",
        "candidateCount",
        Parameter::Integer,
        CHAT | TEXT,
    ),
    (
        "logprobs",
        "logprobs",
        Parameter::Integer,
        TEXT | TEXT_STREAMING,
    ),
    (
        "presence_penalty",
        "presencePenalty",
        Parameter::Number,
        TEXT | TEXT_STREAMING,
    ),
    (
        "frequency_penalty",
        "frequencyPenalty",
        Parameter::Number,
        TEXT | TEXT_STREAMING,
    ),
    (
        "logit_bias",
        "logitBias",
        Parameter::Object,
        TEXT | TEXT_STREAMING,
    ),
    ("seed", "seed", Parameter::Integer, TEXT | TEXT_STREAMING),
];

impl PredictCall {
    const fn mask(self) -> u8 {
        match self {
            Self::Chat => CHAT,
            Self::ChatStreaming => CHAT_STREAMING,
            Self::Text => TEXT,
            Self::TextStreaming => TEXT_STREAMING,
        }
    }

    // the SDK's text predict methods default max_output_tokens to 128; the chat methods leave it unset
    const fn max_decode_steps(self) -> Option<i32> {
        match self {
            Self::Text | Self::TextStreaming => Some(128),
            Self::Chat | Self::ChatStreaming => None,
        }
    }
}

fn parameter(call: PredictCall, name: &str) -> Option<(&'static str, Parameter)> {
    PARAMETERS.iter().find_map(|(source, wire, kind, mask)| {
        (*source == name && *mask & call.mask() != 0).then_some((*wire, *kind))
    })
}

pub fn parameters(options: &Extra, call: PredictCall) -> Result<Extra, PredictionError> {
    let mut parameters = Extra::new();
    if let Some(default) = call.max_decode_steps() {
        parameters.insert(
            String::from("maxDecodeSteps"),
            Json::Number(Number::from(default)),
        );
    }
    for (name, value) in options {
        let Some((wire, kind)) = parameter(call, name) else {
            return Err(PredictionError::UnsupportedOption { name: name.clone() });
        };
        parameters.insert(String::from(wire), convert(name, kind, value)?);
    }
    Ok(parameters)
}

fn convert(name: &str, kind: Parameter, value: &Json) -> Result<Json, PredictionError> {
    match kind {
        Parameter::Integer => {
            if let Json::Number(number) = value
                && number.as_i64().is_some()
            {
                return Ok(Json::Number(number.clone()));
            }
            Err(PredictionError::OptionType {
                name: name.to_owned(),
            })
        }
        Parameter::Number => {
            let Json::Number(number) = value else {
                return Err(PredictionError::OptionType {
                    name: name.to_owned(),
                });
            };
            let value = number.as_f64().ok_or_else(|| PredictionError::OptionType {
                name: name.to_owned(),
            })?;
            Number::from_f64(value)
                .map(Json::Number)
                .ok_or_else(|| PredictionError::OptionType {
                    name: name.to_owned(),
                })
        }
        Parameter::Texts => match value {
            Json::List(items) if items.iter().all(|item| matches!(item, Json::Text(_))) => {
                Ok(value.clone())
            }
            _ => Err(PredictionError::OptionType {
                name: name.to_owned(),
            }),
        },
        Parameter::Object => match value {
            Json::Object(_fields) => Ok(value.clone()),
            _ => Err(PredictionError::OptionType {
                name: name.to_owned(),
            }),
        },
    }
}
