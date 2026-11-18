use std::collections::BTreeMap;

use betterlite_bedrock::agentcore_stream::transport::HandlerKind;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "scenario", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    TransportArguments { runs: Vec<Run> },
    WrapperIdentity { runs: Vec<Run> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Run {
    pub(crate) wrapper: Wrapper,
    pub(crate) client: Client,
    pub(crate) model: String,
    pub(crate) api_base: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) data: IndexMap<String, Json>,
    pub(crate) messages: Json,
    #[serde(default)]
    pub(crate) json_mode: Option<bool>,
    #[serde(default)]
    pub(crate) signed_json_body: Option<String>,
    #[serde(default)]
    pub(crate) stream_chunk_size: Option<u64>,
    #[serde(default)]
    pub(crate) response_headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Wrapper {
    Async,
    Sync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Client {
    None,
    Async,
    Sync,
}

impl Run {
    // A real POST needs a fixture of the wrapper's own kind, the pair LiteLLM's typed client argument allows.
    pub(crate) fn identity_kind(&self) -> Result<HandlerKind, &'static str> {
        match (self.wrapper, self.client) {
            (Wrapper::Async, Client::Async) => Ok(HandlerKind::Async),
            (Wrapper::Sync, Client::Sync) => Ok(HandlerKind::Sync),
            (Wrapper::Async | Wrapper::Sync, Client::None | Client::Async | Client::Sync) => {
                Err("a wrapper identity run needs a client of the wrapper's kind")
            }
        }
    }
}
