mod call;
mod client;
mod config;
mod harness;
mod project;

use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::PoisonError;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Remix,
    AvideoRemix,
    List,
    AvideoList,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    litellm_call_id: String,
    status: u16,
    body: String,
    #[serde(default)]
    request_timeout_seconds: Option<f64>,
    #[serde(default)]
    video_id: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    after: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    order: Option<String>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    // absent: the signature's 600; null: an explicit None; a number: that many seconds
    #[serde(default)]
    timeout: Option<Value>,
    // serde folds a null into the absent case, so a null case marks itself here
    #[serde(default)]
    timeout_explicitly_none: bool,
    #[serde(default)]
    extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    extra_body: Option<Value>,
    #[serde(default)]
    extra_query: Option<Value>,
    #[serde(default)]
    mock_response: Option<Value>,
    #[serde(default)]
    user: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let mut bench = harness::Bench::new(input.status, &input.body, &input.litellm_call_id)?;
        let outcome = call::run(input, &mut bench)?;
        let wanted = project::wanted_headers(&input.extra_headers);
        let request = {
            let requests = bench
                .source
                .wire
                .requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            project::request(requests.first(), &wanted)
        };
        let mut answer = Map::new();
        answer.insert("request".to_owned(), request);
        answer.insert("response".to_owned(), outcome.response);
        answer.insert("error".to_owned(), outcome.error);
        answer.insert("logging".to_owned(), project::logging(&bench.state));
        Ok(Value::Object(answer))
    }
}

pub(crate) fn failed(error: &dyn Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: "OSError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
