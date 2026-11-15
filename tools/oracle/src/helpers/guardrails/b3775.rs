mod headroom;
mod hiddenlayer;
mod scripted;

use serde::Deserialize;
use serde::de::Error as _;
use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

// an internally tagged enum buffers the payload, where a RawValue response cannot be read
#[derive(Deserialize)]
#[serde(try_from = "Value")]
pub(crate) enum Input {
    ShouldRun(Box<headroom::ShouldRun>),
    Plan(Box<headroom::Plan>),
    Hiddenlayer(Box<hiddenlayer::Input>),
}

impl TryFrom<Value> for Input {
    type Error = serde_json::Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let Value::Object(mut fields) = value else {
            return Err(serde_json::Error::custom(
                "a b3775 helper input is a mapping",
            ));
        };
        let Some(tag) = fields.remove("kind") else {
            return Err(serde_json::Error::custom(
                "the b3775 helper input names no kind",
            ));
        };
        let Some(kind) = tag.as_str() else {
            return Err(serde_json::Error::custom(
                "the b3775 helper kind is not a string",
            ));
        };
        let payload = Value::Object(fields);
        match kind {
            "should_run" => Ok(Self::ShouldRun(Box::new(serde_json::from_value(payload)?))),
            "plan" => Ok(Self::Plan(Box::new(serde_json::from_value(payload)?))),
            "hiddenlayer" => Ok(Self::Hiddenlayer(Box::new(serde_json::from_value(
                payload,
            )?))),
            other => Err(serde_json::Error::custom(format!(
                "unknown b3775 helper kind: {other}"
            ))),
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ShouldRun(item) => headroom::should_run(item),
            Input::Plan(item) => headroom::plan(item),
            Input::Hiddenlayer(item) => hiddenlayer::run(item),
        }
    }
}
