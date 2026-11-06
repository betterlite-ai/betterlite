use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::error::ExitFailure;
use super::runtime::McpRuntime;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportType {
    Sse,
    #[default]
    Http,
    Stdio,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct StdioConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StdioServerParameters {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    // The SDK's inherited variables, merged under env when the server starts
    pub defaults: BTreeMap<String, String>,
}

pub trait TransportContext: Sized + Send + 'static {
    type Streams: Send + 'static;
    type Error: Error + Send + Sync + 'static;

    fn enter(&mut self) -> impl Future<Output = Result<Self::Streams, Self::Error>> + Send;

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<Self::Error>>> + Send + 'static;
}

pub struct TransportContexts<Runtime>
where
    Runtime: McpRuntime,
{
    pub transport: Runtime::Transport,
    pub http_client: Option<Arc<Runtime::HttpClient>>,
}
