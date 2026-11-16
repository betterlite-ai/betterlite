use crate::server_payload::request::OrderedMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde::Serialize;
use serde::de::Error as _;
use serde_with::rust::double_option;
mod tests;
pub struct ConnectorEntry {
    pub name: Option<String>,
    pub entry_type: Option<String>,
    pub url: Option<String>,
    pub authorization_token: Option<SecretString>,
    pub headers: Option<OrderedMap>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: OrderedMap,
    pub description: Option<String>,
}
pub enum ConnectorServers {
    Mapping(IndexMap<String, ConnectorEntry>),
    List(Vec<ConnectorEntry>),
}
pub struct ConnectorImportRequest {
    pub mcp_servers: ConnectorServers,
}
type Maybe<T> = Option<Option<T>>;
#[derive(Deserialize)]
struct RawConnectorImportRequest {
    #[serde(default, rename = "mcp_servers", with = "double_option")]
    mcp_servers_snake: Maybe<RawServers>,
    #[serde(default, rename = "mcpServers", with = "double_option")]
    mcp_servers_camel: Maybe<RawServers>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum RawServers {
    Mapping(IndexMap<String, RawConnectorEntry>),
    List(Vec<RawConnectorEntry>),
}
#[derive(Deserialize)]
struct RawConnectorEntry {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default, rename = "authorization_token", with = "double_option")]
    authorization_token_snake: Maybe<SecretString>,
    #[serde(default, rename = "authorizationToken", with = "double_option")]
    authorization_token_camel: Maybe<SecretString>,
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
        let Some(servers) = raw.mcp_servers_snake.or(raw.mcp_servers_camel).flatten() else {
            return Err(serde_json::Error::custom(
                "mcp_servers must be an object or a list of connector entries",
            ));
        };
        let mcp_servers = match servers {
            RawServers::Mapping(entries) => ConnectorServers::Mapping(
                entries
                    .into_iter()
                    .map(|(name, entry)| (name, entry.into_entry()))
                    .collect(),
            ),
            RawServers::List(entries) => ConnectorServers::List(
                entries
                    .into_iter()
                    .map(RawConnectorEntry::into_entry)
                    .collect(),
            ),
        };
        Ok(Self { mcp_servers })
    }
}
impl RawConnectorEntry {
    fn into_entry(self) -> ConnectorEntry {
        let snake = self.authorization_token_snake;
        let camel = self.authorization_token_camel;
        let authorization_token = snake.or(camel).flatten();
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
