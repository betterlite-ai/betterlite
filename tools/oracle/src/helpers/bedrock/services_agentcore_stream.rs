use betterlite_bedrock::agentcore_stream::StreamRequest;
use betterlite_bedrock::agentcore_stream::transport::HandlerKind;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use uuid::Uuid;

use self::fixture::{Fixture, drain, header_map};
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
    supplied: Option<Kind>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Sync,
    Async,
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

async fn run_scenario(
    model: &str,
    ids: &[Uuid],
    scenario: &Scenario,
    kind: HandlerKind,
) -> Result<Value, Box<HelperError>> {
    let headers = header_map(&scenario.request_headers)?;
    let fixture = Fixture::new(kind, scenario, ids)?;
    let request = StreamRequest {
        model,
        api_base: &scenario.api_base,
        headers,
        data: &scenario.data,
        signed_json_body: scenario.signed_json_body.as_deref(),
        messages: &scenario.messages,
    };
    let opened = fixture.open(&request, kind).await;
    let (provider, emitted) = match opened {
        Ok(mut stream) => {
            let provider = stream.provider;
            (Some(provider), drain(&mut stream).await)
        }
        Err(error) => (None, Err(error)),
    };
    let handler = if scenario.supplied == Some(kind_of(kind)) {
        "supplied"
    } else {
        "default"
    };
    Ok(outcome(&fixture, handler, provider, &emitted, scenario))
}

const fn kind_of(kind: HandlerKind) -> Kind {
    match kind {
        HandlerKind::Sync => Kind::Sync,
        HandlerKind::Async => Kind::Async,
    }
}

const fn handler_kind(kind: Kind) -> HandlerKind {
    match kind {
        Kind::Sync => HandlerKind::Sync,
        Kind::Async => HandlerKind::Async,
    }
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
