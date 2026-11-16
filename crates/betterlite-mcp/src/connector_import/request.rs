use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use serde::de::Error as _;

use crate::server_payload::request::OrderedMap;

mod tests;

pub type ConnectorMap = IndexMap<String, ConnectorEntry>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectorEntry {
    pub name: Option<String>,
    pub entry_type: Option<String>,
    pub url: Option<String>,
    pub authorization_token: Option<String>,
    pub headers: Option<OrderedMap>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: OrderedMap,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectorServers {
    Mapping(ConnectorMap),
    List(Vec<ConnectorEntry>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectorImportRequest {
    pub mcp_servers: ConnectorServers,
}

#[derive(Debug, Deserialize)]
struct RawConnectorImportRequest {
    #[serde(default, rename = "mcp_servers")]
    mcp_servers_snake: Option<Option<RawServers>>,
    #[serde(default, rename = "mcpServers")]
    mcp_servers_camel: Option<Option<RawServers>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawServers {
    Mapping(IndexMap<String, RawConnectorEntry>),
    List(Vec<RawConnectorEntry>),
}

#[derive(Debug, Deserialize)]
struct RawConnectorEntry {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default, rename = "authorization_token")]
    authorization_token_snake: Option<Option<String>>,
    #[serde(default, rename = "authorizationToken")]
    authorization_token_camel: Option<Option<String>>,
    #[serde(default)]
    headers: Option<OrderedMap>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: OrderedMap,
    #[serde(default)]
    description: Option<String>,
}

impl ConnectorImportRequest {
    pub fn from_slice(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let raw: RawConnectorImportRequest = serde_json::from_slice(bytes)?;
        let servers = match (raw.mcp_servers_snake, raw.mcp_servers_camel) {
            (Some(value), _) => value,
            (None, Some(value)) => value,
            (None, None) => None,
        };
        let Some(servers) = servers else {
            return Err(serde_json::Error::custom(
                "mcp_servers must be an object or a list of connector entries",
            ));
        };
        let mcp_servers = match servers {
            RawServers::Mapping(entries) => ConnectorServers::Mapping(
                entries
                    .into_iter()
                    .map(|(name, entry)| (name, entry.into_entry()))
                    .collect::<ConnectorMap>(),
            ),
            RawServers::List(entries) => ConnectorServers::List(
                entries
                    .into_iter()
                    .map(RawConnectorEntry::into_entry)
                    .collect::<Vec<ConnectorEntry>>(),
            ),
        };
        Ok(Self { mcp_servers })
    }
}

impl RawConnectorEntry {
    fn into_entry(self) -> ConnectorEntry {
        let authorization_token = match (
            self.authorization_token_snake,
            self.authorization_token_camel,
        ) {
            (Some(value), _) => value,
            (None, Some(value)) => value,
            (None, None) => None,
        };
        ConnectorEntry {
            name: self.name,
            entry_type: self.kind,
            url: self.url,
            authorization_token,
            headers: self.headers,
            command: self.command,
            args: self.args,
            env: self.env,
            description: self.description,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MCPConnectorImportResult {
    pub name: String,
    pub server_id: String,
    pub alias: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MCPConnectorImportSkipped {
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MCPConnectorImportFailure {
    pub name: String,
    pub error: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MCPConnectorImportResponse {
    pub imported: Vec<MCPConnectorImportResult>,
    pub skipped: Vec<MCPConnectorImportSkipped>,
    pub errors: Vec<MCPConnectorImportFailure>,
}
