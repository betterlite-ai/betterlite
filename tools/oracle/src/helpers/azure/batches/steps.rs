use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use betterlite_azure::batches::client::{ClientSettings, Connection};
use betterlite_azure::batches::error::AzureBatchesError;
use betterlite_azure::batches::{Operation, Reply, call};
use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::endpoints::projection::{
    ClientKind, ClientOutcome, LiteLlmParams, SessionKind, SuppliedClient,
};
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_openai::batches::records::{BatchCreate, BatchQuery};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::fixture::{Cache, Log, Materializer, Planned, Sources};
use super::reports::{projection, raised, value_error};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Project,
    Create,
    Retrieve,
    Cancel,
    List,
}

// The handler takes timeout and max_retries beside the api arguments and never passes them on, so they stay unread
#[derive(Clone, Deserialize)]
struct Args {
    api_key: Option<String>,
    api_base: Option<String>,
    api_version: Option<String>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BuilderMode {
    #[default]
    Default,
    None,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    action: Action,
    args: Args,
    #[serde(default)]
    is_async: bool,
    client: Option<String>,
    #[serde(default)]
    builder: BuilderMode,
    data: Option<BatchCreate>,
    query: Option<BatchQuery>,
    params: Option<LiteLlmParams>,
    after: Option<String>,
    limit: Option<i64>,
    pub(crate) reply: Option<Planned>,
}

fn supplied_kind(name: &str) -> Option<ClientKind> {
    match name {
        "openai" => Some(ClientKind::OpenAi),
        "async_openai" => Some(ClientKind::AsyncOpenAi),
        "azure_openai" => Some(ClientKind::AzureOpenAi),
        "async_azure_openai" => Some(ClientKind::AsyncAzureOpenAi),
        _ => None,
    }
}

fn reply_value(result: Result<Reply, AzureBatchesError>) -> Value {
    match result {
        Ok(Reply::Batch(batch)) => json!({ "returned": batch }),
        Ok(Reply::Page(page)) => json!({ "returned": page }),
        Err(error) => raised(&error),
    }
}

// A client builder that answers None makes every handler action report the missing client first
fn project_value(
    outcome: &ClientOutcome<'_>,
    connection: &Connection<'_, '_, Materializer>,
) -> Value {
    match connection.materializer.materialize(outcome) {
        Ok(Some(_)) => json!({ "returned": projection(outcome) }),
        Ok(None) => raised(&AzureBatchesError::ClientMissing),
        Err(source) => raised(&AzureBatchesError::Sdk { source }),
    }
}

fn perform(
    step: &Step,
    mut connection: Connection<'_, '_, Materializer>,
    runtime: &Runtime,
) -> Value {
    match step.action {
        Action::Project => match connection.get_openai_client() {
            Ok(outcome) => project_value(&outcome, &connection),
            Err(error) => raised(&error),
        },
        Action::Create => match step.data.as_ref() {
            Some(data) => reply_value(runtime.block_on(call(connection, Operation::Create(data)))),
            None => value_error("a create step carries no data"),
        },
        Action::Retrieve => match step.query.as_ref() {
            Some(query) => {
                reply_value(runtime.block_on(call(connection, Operation::Retrieve(query))))
            }
            None => value_error("a retrieve step carries no query"),
        },
        Action::Cancel => match step.query.as_ref() {
            Some(query) => {
                reply_value(runtime.block_on(call(connection, Operation::Cancel(query))))
            }
            None => value_error("a cancel step carries no query"),
        },
        Action::List => reply_value(runtime.block_on(call(
            connection,
            Operation::List {
                after: step.after.as_deref(),
                limit: step.limit,
            },
        ))),
    }
}

fn client_settings(step: &Step) -> ClientSettings {
    ClientSettings {
        api_key: step.args.api_key.clone().map(SecretString::from),
        api_base: step.args.api_base.clone(),
        api_version: step.args.api_version.clone(),
    }
}

fn supplied(step: &Step) -> Option<SuppliedClient> {
    step.client
        .as_deref()
        .and_then(supplied_kind)
        .map(|kind| SuppliedClient {
            kind,
            custom_query: None,
        })
}

pub(crate) fn run_step(runtime: &Runtime, step: &Step) -> Value {
    let calls = Log::default();
    let materializer = Materializer {
        mode: step.builder,
        reply: step.reply.clone(),
        log: Arc::clone(&calls),
    };
    let endpoints = EndpointSettings::default();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let sources = Sources;
    let mut cache = Cache;
    let mut refresh = RefreshProviderCache::new(&sources);
    let connection = Connection {
        settings: client_settings(step),
        params: step.params.clone().unwrap_or_default(),
        provider: None,
        supplied: supplied(step),
        is_async: step.is_async,
        endpoints: &endpoints,
        environment: &environment,
        sources: &sources,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
        cache: &mut cache,
        refresh: &mut refresh,
        materializer: &materializer,
    };
    let outcome = perform(step, connection, runtime);
    json!({
        "outcome": outcome,
        "calls": calls.lock().unwrap_or_else(PoisonError::into_inner).clone(),
    })
}
