mod fixture;
mod harness;
mod projection;
mod records;
mod server;

use std::collections::BTreeMap;
use std::net::SocketAddr;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_core::api::agents::item::error::AgentItemFailureError;
use betterlite_core::api::agents::item::{
    AgentItemDeps, AgentItemRequest, adelete, aget, alist_versions, delete, get, list_versions,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Number, Value, json};
use tokio::runtime::Runtime;

use crate::helpers::{Helper, HelperError, HelperResult};

use fixture::{FixtureConfig, FixtureError};
use harness::Harness;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Get,
    Delete,
    ListVersions,
    Aget,
    Adelete,
    AlistVersions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyInput {
    status: u16,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    name: String,
    api_key: String,
    litellm_call_id: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    page_size: Option<i64>,
    #[serde(default)]
    page_token: Option<String>,
    reply: ReplyInput,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let (address, captured) = server::serve(input.reply.status, &input.reply.body)?;
        let mut harness = Harness::new(&input.litellm_call_id)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failed(&error))?;
        let kwargs = kwargs_of(input, &address);
        let request = AgentItemRequest {
            name: &input.name,
            custom_llm_provider: input.provider.as_deref(),
            extra_headers: input.extra_headers.clone(),
            timeout: None,
            request_timeout: RequestTimeout {
                seconds: 6000.0,
                explicitly_set: false,
            },
            kwargs: &kwargs,
        };
        let deps = AgentItemDeps {
            clients: &mut harness.clients,
            logging: &mut harness.state,
            http: &mut harness.record,
            redactor: &harness.redactor,
        };
        let outcome = operate(input.operation, &FixtureConfig, deps, &request, &runtime);
        let head = captured.join().unwrap_or_default();
        let mut answer = Map::new();
        answer.insert(
            "request".to_owned(),
            projection::request(&head, &wanted(input)),
        );
        answer.insert("logging".to_owned(), projection::logging(&harness));
        match outcome {
            Ok(value) => answer.insert("response".to_owned(), value),
            Err(error) => answer.insert("error".to_owned(), projection::error(&error.class)),
        };
        Ok(Value::Object(answer))
    }
}

fn operate(
    operation: Operation,
    config: &FixtureConfig,
    deps: AgentItemDeps<'_>,
    request: &AgentItemRequest<'_>,
    runtime: &Runtime,
) -> Result<Value, AgentItemFailureError<FixtureError>> {
    match operation {
        Operation::Get => get(config, deps, request)
            .map(|value| json!({"id": &value.id, "name": &value.name})),
        Operation::Delete => delete(config, deps, request)
            .map(|value| json!({"name": &value.name, "deleted": value.deleted})),
        Operation::ListVersions => list_versions(config, deps, request).map(|value| {
            json!({"agent_versions": &value.agent_versions, "next_page_token": &value.next_page_token})
        }),
        Operation::Aget => runtime
            .block_on(aget(config, deps, request))
            .map(|value| json!({"id": &value.id, "name": &value.name})),
        Operation::Adelete => runtime
            .block_on(adelete(config, deps, request))
            .map(|value| json!({"name": &value.name, "deleted": value.deleted})),
        Operation::AlistVersions => runtime.block_on(alist_versions(config, deps, request)).map(
            |value| json!({"agent_versions": &value.agent_versions, "next_page_token": &value.next_page_token}),
        ),
    }
}

// LiteLLM hands the provider and the credentials to the handler through litellm_params
fn kwargs_of(input: &Input, address: &SocketAddr) -> IndexMap<String, Json> {
    let mut kwargs = IndexMap::new();
    kwargs.insert(
        "api_base".to_owned(),
        Json::Text(format!("http://{address}")),
    );
    kwargs.insert("api_key".to_owned(), Json::Text(input.api_key.clone()));
    kwargs.insert(
        "litellm_call_id".to_owned(),
        Json::Text(input.litellm_call_id.clone()),
    );
    if let Some(size) = input.page_size {
        kwargs.insert("page_size".to_owned(), Json::Number(Number::from(size)));
    }
    if let Some(token) = &input.page_token {
        kwargs.insert("page_token".to_owned(), Json::Text(token.clone()));
    }
    kwargs
}

// the headers every case compares: the provider's own two plus whatever the caller forwarded
fn wanted(input: &Input) -> Vec<String> {
    let mut names = vec!["content-type".to_owned(), "x-goog-api-key".to_owned()];
    for name in input.extra_headers.keys() {
        let lowered = name.to_lowercase();
        if !names.contains(&lowered) {
            names.push(lowered);
        }
    }
    names
}

pub(super) fn failed(error: &dyn std::fmt::Display) -> Box<HelperError> {
    let message = error.to_string();
    Box::new(HelperError {
        class: "OSError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
