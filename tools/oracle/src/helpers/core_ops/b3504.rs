use std::collections::{BTreeMap, BTreeSet};

use betterlite_core::api::agents::collection::creation::{acreate, create};
use betterlite_core::api::agents::collection::listing::{alist, list};
use betterlite_core::api::agents::collection::{
    AgentCollectionHandled, AgentCreation, AgentListing,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

mod gemini;
mod support;

use support::Fixture;
use support::server::Server;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Create,
    Acreate,
    List,
    Alist,
}

// the arguments LiteLLM's create_agent, acreate_agent, list_agents and alist_agents take, plus the reply the
// mock provider answers with; the helpers inject api_base and the credentials both sides share
#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    base_agent: Option<String>,
    #[serde(default)]
    instructions: Option<String>,
    #[serde(default)]
    base_environment: Option<Json>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    kwargs: IndexMap<String, Json>,
    reply: support::Reply,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let server = Server::start(&input.reply)?;
        let kwargs = injected(input, &server);
        let mut fixture = Fixture::new()?;
        let result = match input.function {
            Function::Create => run_create(input, &kwargs, &mut fixture)?,
            Function::List => run_list(input, &kwargs, &mut fixture)?,
            Function::Acreate => run_acreate(input, &kwargs, &mut fixture)?,
            Function::Alist => run_alist(input, &kwargs, &mut fixture)?,
        };
        Ok(support::answer_value(
            &server,
            &wanted_headers(input),
            result,
            &fixture.logging,
        ))
    }
}

fn injected(input: &Input, server: &Server) -> IndexMap<String, Json> {
    let mut kwargs = input.kwargs.clone();
    kwargs
        .entry("api_base".to_owned())
        .or_insert_with(|| Json::Text(server.base()));
    kwargs
        .entry("api_key".to_owned())
        .or_insert_with(|| Json::Text("key-1".to_owned()));
    kwargs
        .entry("litellm_call_id".to_owned())
        .or_insert_with(|| Json::Text("call-1".to_owned()));
    kwargs
}

fn wanted_headers(input: &Input) -> BTreeSet<String> {
    input
        .extra_headers
        .keys()
        .map(|name| name.to_lowercase())
        .collect()
}

fn creation<'call>(
    input: &'call Input,
    kwargs: &'call IndexMap<String, Json>,
) -> AgentCreation<'call> {
    AgentCreation {
        name: input.name.as_deref().unwrap_or(""),
        base_agent: input.base_agent.as_deref(),
        instructions: input.instructions.as_deref(),
        base_environment: input.base_environment.as_ref(),
        custom_llm_provider: input.custom_llm_provider.as_deref(),
        extra_headers: input.extra_headers.clone(),
        extra_body: input.extra_body.as_ref(),
        timeout: None,
        kwargs,
    }
}

fn listing<'call>(
    input: &'call Input,
    kwargs: &'call IndexMap<String, Json>,
) -> AgentListing<'call> {
    AgentListing {
        custom_llm_provider: input.custom_llm_provider.as_deref(),
        extra_headers: input.extra_headers.clone(),
        timeout: None,
        kwargs,
    }
}

fn run_create(
    input: &Input,
    kwargs: &IndexMap<String, Json>,
    fixture: &mut Fixture,
) -> Result<Value, Box<HelperError>> {
    match create(fixture.collection(), creation(input, kwargs)) {
        Ok(AgentCollectionHandled::Done(value)) => Ok(value),
        Ok(AgentCollectionHandled::Pending(_pending)) => Err(support::helper_failure(
            "the create was unexpectedly pending",
        )),
        Err(error) => Err(support::raised(&error)),
    }
}

fn run_list(
    input: &Input,
    kwargs: &IndexMap<String, Json>,
    fixture: &mut Fixture,
) -> Result<Value, Box<HelperError>> {
    match list(fixture.collection(), listing(input, kwargs)) {
        Ok(AgentCollectionHandled::Done(value)) => Ok(value),
        Ok(AgentCollectionHandled::Pending(_pending)) => {
            Err(support::helper_failure("the list was unexpectedly pending"))
        }
        Err(error) => Err(support::raised(&error)),
    }
}

fn run_acreate(
    input: &Input,
    kwargs: &IndexMap<String, Json>,
    fixture: &mut Fixture,
) -> Result<Value, Box<HelperError>> {
    let runtime = runtime()?;
    runtime
        .block_on(acreate(fixture.collection(), creation(input, kwargs)))
        .map_err(|error| support::raised(&error))
}

fn run_alist(
    input: &Input,
    kwargs: &IndexMap<String, Json>,
    fixture: &mut Fixture,
) -> Result<Value, Box<HelperError>> {
    let runtime = runtime()?;
    runtime
        .block_on(alist(fixture.collection(), listing(input, kwargs)))
        .map_err(|error| support::raised(&error))
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(support::helper_failure)
}
