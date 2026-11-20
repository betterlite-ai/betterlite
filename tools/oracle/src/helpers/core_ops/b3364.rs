mod fixture;
mod input;
mod projection;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_core::api::container_collection_http::{
    ContainerCollectionError, ContainerCreateCall, ContainerListCall, container_create_handler,
    container_list_handler,
};
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery,
};
use betterlite_openai::containers::container_file_http::transport::ContainerHttpReply;
use serde_json::{Map, Value};
use tokio::runtime::Builder;

use self::fixture::{Fake, RecordingHook, config, deadline, failure, logging_state};
use self::input::{Input, Operation};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(&error))?;
        let stopped = input.failure.map(|_| deadline(&runtime)).transpose()?;
        let reply = ContainerHttpReply {
            status: input.status,
            headers: input.reply_headers.clone(),
            body: input.body.as_bytes().to_vec(),
        };
        let source = Fake::new(reply, stopped);
        let provider = config(&BTreeMap::new())?;
        let logging = Mutex::new(logging_state()?);
        let hook = RecordingHook::default();
        let harness = Harness {
            provider: &provider,
            source: &source,
            logging: &logging,
            hook: &hook,
        };
        let outcome = match input.operation {
            Operation::Create => create(input, &harness),
            Operation::List => list(input, &harness),
        };
        answer(outcome, &source, &hook)
    }
}

// The one compared record: what went on the wire, what a callback saw, and either the model or the exception
fn answer(
    outcome: Result<Value, ContainerCollectionError>,
    source: &Fake,
    hook: &RecordingHook,
) -> HelperResult {
    let conclusion = match outcome {
        Ok(value) => (String::from("response"), value),
        Err(ContainerCollectionError::Provider(exception)) => {
            (String::from("error"), projection::provider(&exception))
        }
        Err(other) => return Err(failure(&other)),
    };
    let mut output = Map::new();
    output.insert(
        String::from("request"),
        projection::request(&source.requests())?,
    );
    output.insert(String::from("log"), projection::log(&hook.calls())?);
    output.insert(conclusion.0, conclusion.1);
    Ok(Value::Object(output))
}

// What a create or a list needs past the case input, so neither entry point takes six arguments
struct Harness<'run> {
    provider: &'run dyn BaseContainerConfig,
    source: &'run Fake,
    logging: &'run Mutex<LoggingState>,
    hook: &'run RecordingHook,
}

// The two litellm_params a case sets; every transform reads only these two, as LiteLLM's do
fn litellm_params(input: &Input) -> Map<String, Value> {
    let mut params = Map::new();
    if let Some(key) = &input.api_key {
        params.insert(String::from("api_key"), Value::String(key.clone()));
    }
    if let Some(base) = &input.api_base {
        params.insert(String::from("api_base"), Value::String(base.clone()));
    }
    params
}

fn create(input: &Input, harness: &Harness<'_>) -> Result<Value, ContainerCollectionError> {
    let params = litellm_params(input);
    let call = ContainerCreateCall {
        name: &input.name,
        params: &input.create_params,
        config: harness.provider,
        litellm_params: &params,
        logging: harness.logging,
        extra_headers: None,
        timeout: input.timeout.map(Duration::from_secs),
    };
    let record = container_create_handler(&call, harness.source, None, harness.hook)?;
    Ok(projection::container(&record))
}

fn list(input: &Input, harness: &Harness<'_>) -> Result<Value, ContainerCollectionError> {
    let params = litellm_params(input);
    let call = ContainerListCall {
        config: harness.provider,
        litellm_params: &params,
        logging: harness.logging,
        extra_headers: None,
        query: ContainerListQuery {
            after: input.after.as_deref(),
            limit: input.limit,
            order: input.order.as_deref(),
            extra_query: None,
        },
        timeout: None,
    };
    let page = container_list_handler(&call, harness.source, None, harness.hook)?;
    Ok(projection::page(&page))
}
