use std::sync::{Arc, PoisonError};

use betterlite_bedrock::agentcore_stream::adapter::open;
use betterlite_bedrock::agentcore_stream::transport::{HandlerKind, StreamHandler};
use betterlite_bedrock::agentcore_stream::{IdSource, StreamDeps, StreamRequest};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use uuid::Uuid;

use self::fixture::{FixedIds, FixtureHandler, Recorder, drain, header_map};
use self::project::outcome;
use crate::helpers::HelperError;

mod fixture;
mod project;
mod tests;

const DEFAULT_API_BASE: &str = "https://bedrock-agentcore.example/runtime";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    ids: Vec<String>,
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    #[serde(default)]
    reads: Vec<String>,
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default = "default_api_base")]
    api_base: String,
    #[serde(default)]
    request_headers: Vec<(String, String)>,
    #[serde(default = "default_data")]
    data: Json,
    #[serde(default)]
    signed_json_body: Option<Vec<u8>>,
    #[serde(default = "default_messages")]
    messages: Json,
    #[serde(default)]
    supplied: Option<String>,
}

const fn default_status() -> u16 {
    200
}

fn default_api_base() -> String {
    DEFAULT_API_BASE.to_owned()
}

fn default_data() -> Json {
    Json::Object(IndexMap::default())
}

const fn default_messages() -> Json {
    Json::List(Vec::new())
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let ids = parse_ids(&input.ids)?;
        Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(error.to_string()))?
            .block_on(execute(input, &ids))
    }
}

async fn execute(input: &Input, ids: &[Uuid]) -> crate::helpers::HelperResult {
    let mut sync = Vec::new();
    let mut asynchronous = Vec::new();
    for scenario in &input.scenarios {
        sync.push(run_scenario(&input.model, ids, scenario, HandlerKind::Sync).await?);
        asynchronous.push(run_scenario(&input.model, ids, scenario, HandlerKind::Async).await?);
    }
    Ok(json!({ "sync": sync, "async": asynchronous }))
}

type Handlers = (Option<Arc<FixtureHandler>>, Arc<FixtureHandler>);

fn build_handlers(
    scenario: &Scenario,
    kind: HandlerKind,
    recorder: &Arc<Recorder>,
) -> Result<Handlers, Box<HelperError>> {
    let supplied_kind = match scenario.supplied.as_deref() {
        Some("sync") => Some(HandlerKind::Sync),
        Some("async") => Some(HandlerKind::Async),
        Some(other) => return Err(failure(format!("unknown supplied client kind {other}"))),
        None => None,
    };
    let supplied = supplied_kind
        .map(|chosen| FixtureHandler::new(chosen, "supplied", recorder, scenario))
        .transpose()?
        .map(Arc::new);
    let default = FixtureHandler::new(kind, "default", recorder, scenario)?;
    Ok((supplied, Arc::new(default)))
}

fn build_deps<'deps>(
    handlers: &Handlers,
    ids: &Arc<FixedIds>,
    logger: &'deps Recorder,
) -> StreamDeps<'deps> {
    let supplied: Option<Arc<dyn StreamHandler>> = handlers.0.as_ref().map(|handler| {
        let cloned: Arc<dyn StreamHandler> = Arc::<FixtureHandler>::clone(handler);
        cloned
    });
    let default: Arc<dyn StreamHandler> = Arc::<FixtureHandler>::clone(&handlers.1);
    let ids_handler: Arc<dyn IdSource> = Arc::<FixedIds>::clone(ids);
    StreamDeps {
        supplied,
        default,
        ids: ids_handler,
        logger,
    }
}

async fn run_scenario(
    model: &str,
    ids: &[Uuid],
    scenario: &Scenario,
    kind: HandlerKind,
) -> Result<Value, Box<HelperError>> {
    let recorder = Arc::new(Recorder::default());
    let handlers = build_handlers(scenario, kind, &recorder)?;
    let id_source = Arc::new(FixedIds::new(ids));
    let request = StreamRequest {
        model,
        api_base: &scenario.api_base,
        headers: header_map(&scenario.request_headers)?,
        data: &scenario.data,
        signed_json_body: scenario.signed_json_body.as_deref(),
        messages: &scenario.messages,
    };
    let deps = build_deps(&handlers, &id_source, &recorder);
    let (provider, chunks, error) = match open(&request, &deps, kind).await {
        Ok(mut stream) => {
            let provider = stream.provider;
            let (chunks, error) = drain(&mut stream).await;
            (Some(provider), chunks, error)
        }
        Err(error) => (None, Vec::new(), Some(error)),
    };
    let records = recorder
        .records
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    Ok(outcome(provider, &chunks, error.as_ref(), &records))
}

fn parse_ids(values: &[String]) -> Result<Vec<Uuid>, Box<HelperError>> {
    values
        .iter()
        .map(|value| Uuid::parse_str(value).map_err(|error| failure(error.to_string())))
        .collect()
}

pub(super) fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "FixtureError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
