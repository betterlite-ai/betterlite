use std::collections::BTreeMap;

use betterlite_transcribe::media_source::{media_predates_job, s3_media_url};
use betterlite_transcribe::polling::aws_error_type;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function")]
pub(crate) enum Input {
    #[serde(rename = "error_type")]
    ErrorType { body: String },
    #[serde(rename = "s3_url")]
    S3Url { media_uri: String, region: String },
    #[serde(rename = "predates")]
    Predates {
        headers: BTreeMap<String, String>,
        job_created_at: f64,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ErrorType { body } => {
                Ok(aws_error_type(body.as_bytes()).map_or(Value::Null, Value::String))
            }
            Input::S3Url { media_uri, region } => match s3_media_url(media_uri, region) {
                Ok(url) => Ok(url.map_or(Value::Null, Value::String)),
                Err(error) => Err(Box::new(HelperError {
                    class: String::from("InvalidURL"),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })),
            },
            Input::Predates {
                headers,
                job_created_at,
            } => {
                let pairs: Vec<(String, String)> = headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect();
                Ok(Value::Bool(media_predates_job(&pairs, *job_created_at)))
            }
        }
    }
}
