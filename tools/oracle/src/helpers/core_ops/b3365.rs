mod clients;
mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_core::api::container_resource_http::{
    ContainerResourceCall, ContainerResourceHandler, ContainerResourceHttpError, Handled,
};
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::container_contracts::records::{
    ContainerObject, DeleteContainerResult,
};
use betterlite_openai::containers::OpenAiContainerConfig;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::containers::container_file_http::transport::{
    ContainerHttpRequest, ContainerMethod,
};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;
use url::Url;

use self::clients::{RecordingHook, ScriptedSource};
use self::support::{container_config, logging_state};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

type Handler = ContainerResourceHandler<ScriptedSource, RecordingHook>;
type Call<'call> = ContainerResourceCall<'call, ScriptedSource>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyInput {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Failure {
    #[default]
    Answer,
    Timeout,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Retrieve,
    Delete,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    asynchronous: bool,
    container_id: String,
    api_key: String,
    api_base: String,
    #[serde(default)]
    extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    extra_query: IndexMap<String, String>,
    reply: ReplyInput,
    failure: Failure,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        run_case(input)
    }
}

struct Fixture {
    config: OpenAiContainerConfig,
    logging: Mutex<LoggingState>,
    params: Object,
    query: IndexMap<String, ParamValue>,
}

impl Fixture {
    fn new(input: &Input) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            config: container_config()?,
            logging: Mutex::new(logging_state()?),
            params: litellm_params(input),
            query: extra_query(input),
        })
    }

    fn call<'call>(&'call self, input: &'call Input) -> Call<'call> {
        ContainerResourceCall {
            container_id: &input.container_id,
            config: &self.config,
            litellm_params: &self.params,
            logging_obj: &self.logging,
            extra_headers: Some(&input.extra_headers),
            extra_query: Some(&self.query),
            timeout: Some(Duration::from_secs(600)),
            blocking_client: None,
            async_client: None,
        }
    }
}

fn run_case(input: &Input) -> HelperResult {
    let fixture = Fixture::new(input)?;
    let source = ScriptedSource::new(&input.reply, input.failure);
    let hook = RecordingHook::default();
    let handler = ContainerResourceHandler {
        clients: source.clone(),
        logging: hook.clone(),
    };
    let call = fixture.call(input);
    let outcome = match input.operation {
        Operation::Retrieve => retrieve(&handler, &call, input)?,
        Operation::Delete => delete(&handler, &call, input)?,
    };
    let request = source.only_request()?;
    observation(input, &request, hook.record(), outcome)
}

fn retrieve(handler: &Handler, call: &Call<'_>, input: &Input) -> HelperResult {
    match handler.container_retrieve_handler(call, input.asynchronous) {
        Ok(Handled::Done(record)) => Ok(ok_outcome(&container_json(&record))),
        Ok(Handled::Pending(pending)) => {
            let runtime = Builder::new_current_thread().build().or_value_error()?;
            let settled = runtime.block_on(pending);
            Ok(settled.map_or_else(
                |error| error_outcome(&error, &input.reply),
                |record| ok_outcome(&container_json(&record)),
            ))
        }
        Err(error) => Ok(error_outcome(&error, &input.reply)),
    }
}

fn delete(handler: &Handler, call: &Call<'_>, input: &Input) -> HelperResult {
    match handler.container_delete_handler(call, input.asynchronous) {
        Ok(Handled::Done(record)) => Ok(ok_outcome(&deletion_json(&record))),
        Ok(Handled::Pending(pending)) => {
            let runtime = Builder::new_current_thread().build().or_value_error()?;
            let settled = runtime.block_on(pending);
            Ok(settled.map_or_else(
                |error| error_outcome(&error, &input.reply),
                |record| ok_outcome(&deletion_json(&record)),
            ))
        }
        Err(error) => Ok(error_outcome(&error, &input.reply)),
    }
}

fn observation(
    input: &Input,
    request: &ContainerHttpRequest,
    logged: Option<Value>,
    outcome: Value,
) -> HelperResult {
    let mut fields = Map::new();
    fields.insert(String::from("method"), json!(method_text(request.method)));
    fields.insert(String::from("url"), json!(sent_url(request)?));
    fields.insert(
        String::from("headers"),
        header_fields(&request.headers, &wanted_sent(input)),
    );
    fields.insert(String::from("logged"), logged.unwrap_or(Value::Null));
    fields.insert(String::from("outcome"), outcome);
    Ok(Value::Object(fields))
}

fn litellm_params(input: &Input) -> Object {
    let mut params = Object::new();
    params.insert(String::from("api_key"), json!(input.api_key));
    params.insert(String::from("api_base"), json!(input.api_base));
    params
}

fn extra_query(input: &Input) -> IndexMap<String, ParamValue> {
    input
        .extra_query
        .iter()
        .map(|(name, value)| (name.clone(), ParamValue::Text(value.clone())))
        .collect()
}

fn ok_outcome(record: &Value) -> Value {
    json!({"kind": "ok", "record": record})
}

fn error_outcome(error: &ContainerResourceHttpError, reply: &ReplyInput) -> Value {
    match error {
        ContainerResourceHttpError::Provider {
            exception,
            status_code_is_synthesized,
        } => json!({
            "kind": "provider",
            "status": exception.status_code,
            "synthesized": status_code_is_synthesized,
            "message": exception.message,
            "headers": header_fields(&exception.headers, &wanted_reply(reply)),
        }),
        ContainerResourceHttpError::Call(source) => {
            json!({"kind": "call", "error": source.to_string()})
        }
        ContainerResourceHttpError::QueryValue { name } => {
            json!({"kind": "query_value", "name": name})
        }
    }
}

fn container_json(record: &ContainerObject) -> Value {
    json!({
        "id": record.id,
        "object": record.object,
        "created_at": record.created_at,
        "status": record.status,
        "name": record.name,
    })
}

fn deletion_json(record: &DeleteContainerResult) -> Value {
    json!({
        "id": record.id,
        "object": record.object,
        "deleted": record.deleted,
    })
}

const fn method_text(method: ContainerMethod) -> &'static str {
    match method {
        ContainerMethod::Get => "GET",
        ContainerMethod::Delete => "DELETE",
        ContainerMethod::Post => "POST",
    }
}

// GET carries its query beside the URL, and the client merges it in as httpx does
fn sent_url(request: &ContainerHttpRequest) -> Result<String, Box<HelperError>> {
    if request.query.is_empty() {
        return Ok(request.url.clone());
    }
    let mut url = Url::parse(&request.url).or_value_error()?;
    url.query_pairs_mut().extend_pairs(
        request
            .query
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
    );
    Ok(url.to_string())
}

fn wanted_sent(input: &Input) -> BTreeSet<String> {
    let mut wanted: BTreeSet<String> = input
        .extra_headers
        .keys()
        .map(|name| name.to_lowercase())
        .collect();
    wanted.insert(String::from("authorization"));
    wanted
}

fn wanted_reply(reply: &ReplyInput) -> BTreeSet<String> {
    reply
        .headers
        .keys()
        .map(|name| name.to_lowercase())
        .collect()
}

fn header_fields(headers: &BTreeMap<String, String>, wanted: &BTreeSet<String>) -> Value {
    let fields: Map<String, Value> = headers
        .iter()
        .filter(|(name, _)| wanted.contains(&name.to_lowercase()))
        .map(|(name, value)| (name.to_lowercase(), json!(value)))
        .collect();
    Value::Object(fields)
}
