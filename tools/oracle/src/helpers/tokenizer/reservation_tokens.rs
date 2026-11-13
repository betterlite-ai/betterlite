use std::sync::Arc;

use async_trait::async_trait;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::reservation::error::ReservationError;
use betterlite_tokenizer::reservation::schedule::{AccountJob, AccountPool, Counts};
use betterlite_tokenizer::reservation::{
    ParsedRequest, RequestBody, approximate_input_size, count_input_tokens,
    count_input_tokens_for_model,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod fixture;

use fixture::fixture;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    CountForModel {
        request: RequestBody,
        model: String,
    },
    Aggregate {
        request: RequestBody,
        models: Vec<String>,
    },
    Size {
        request: RequestBody,
    },
}

struct InlinePool;

#[async_trait]
impl AccountPool for InlinePool {
    async fn run(&self, job: AccountJob) -> Result<Counts, ReservationError> {
        Ok(job.await)
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        runtime.block_on(run_calls(input))
    }
}

async fn run_calls(input: &Input) -> HelperResult {
    let directory = tempfile::tempdir().map_err(|error| failure(error.to_string()))?;
    let counter = Arc::new(fixture(directory.path())?);
    let mut values = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        values.push(run_call(call, &counter).await?);
    }
    Ok(Value::Array(values))
}

async fn run_call(call: &Call, counter: &Arc<Counter>) -> HelperResult {
    match call {
        Call::CountForModel { request, model } => {
            let tokens = count_input_tokens_for_model(counter.as_ref(), request, model).await;
            Ok(json!(tokens))
        }
        Call::Aggregate { request, models } => {
            let counts = count_input_tokens(
                counter,
                ParsedRequest {
                    body: request,
                    raw_body: None,
                },
                models,
                &InlinePool,
            )
            .await;
            Ok(json!(counts))
        }
        Call::Size { request } => Ok(json!(approximate_input_size(request))),
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
