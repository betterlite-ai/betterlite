use std::collections::{BTreeMap, BTreeSet};

use betterlite_core::api::agents::collection::creation::{acreate, create};
use betterlite_core::api::agents::collection::listing::{alist, list};
use betterlite_core::api::agents::collection::{
    AgentAnswer, AgentCollectionHandled, AgentCreation, AgentListing,
};
use betterlite_models::decisions::Json;
use betterlite_vertex::studio_agents::config::error::GeminiAgentError;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) mod support;

use support::Fixture;
use support::server::Server;
use tokio::runtime::{Builder, Runtime};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Create,
    Acreate,
    List,
    Alist,
}

// the arguments LiteLLM's four entry points take plus the mock provider's reply; the helpers inject api_base
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
    // the case's own environment, so a key set there reaches the settings instead of the process env
    #[serde(default)]
    environment: BTreeMap<String, String>,
    reply: support::Reply,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let server = Server::start(&input.reply)?;
        let kwargs = injected(input, &server);
        let mut fixture = Fixture::new(support::settings(&input.environment))?;
        let result = answer(input, &kwargs, &mut fixture)?;
        Ok(support::answer_value(
            &server,
            &wanted_headers(input),
            &result,
            &fixture.logging,
        ))
    }
}

// each lane builds the call LiteLLM's sync entry point receives, then either asks that entry for a pending call or runs the async one
fn answer(
    input: &Input,
    kwargs: &IndexMap<String, Json>,
    fixture: &mut Fixture,
) -> Result<Value, Box<HelperError>> {
    match input.function {
        Function::Create | Function::Acreate => {
            let call = AgentCreation {
                name: input.name.as_deref().unwrap_or(""),
                base_agent: input.base_agent.as_deref(),
                instructions: input.instructions.as_deref(),
                base_environment: input.base_environment.as_ref(),
                custom_llm_provider: input.custom_llm_provider.as_deref(),
                extra_headers: input.extra_headers.clone(),
                extra_body: input.extra_body.as_ref(),
                timeout: None,
                kwargs,
            };
            if matches!(input.function, Function::Acreate) {
                let answered = runtime()?.block_on(acreate(fixture.collection(), call));
                let record = answered.map_err(|error| support::raised(&error))?;
                return serde_json::to_value(record).map_err(support::helper_failure);
            }
            let handled = create(fixture.collection(), &call);
            finished(handled, "the create was unexpectedly pending")
        }
        Function::List | Function::Alist => {
            let call = AgentListing {
                custom_llm_provider: input.custom_llm_provider.as_deref(),
                extra_headers: input.extra_headers.clone(),
                timeout: None,
                kwargs,
            };
            if matches!(input.function, Function::Alist) {
                let answered = runtime()?.block_on(alist(fixture.collection(), call));
                let record = answered.map_err(|error| support::raised(&error))?;
                return serde_json::to_value(record).map_err(support::helper_failure);
            }
            let handled = list(fixture.collection(), &call);
            finished(handled, "the list was unexpectedly pending")
        }
    }
}

fn finished<Record>(
    answered: AgentAnswer<'_, Record, GeminiAgentError>,
    pending: &'static str,
) -> Result<Value, Box<HelperError>>
where
    Record: Serialize,
{
    match answered {
        Ok(AgentCollectionHandled::Done(record)) => {
            serde_json::to_value(record).map_err(support::helper_failure)
        }
        Ok(AgentCollectionHandled::Pending(_pending)) => Err(support::helper_failure(pending)),
        Err(error) => Err(support::raised(&error)),
    }
}

fn injected(input: &Input, server: &Server) -> IndexMap<String, Json> {
    let mut kwargs = input.kwargs.clone();
    kwargs
        .entry("api_base".to_owned())
        .or_insert_with(|| Json::Text(server.base.clone()));
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

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(support::helper_failure)
}
