use std::collections::BTreeMap;

use betterlite_mcp::credentials::McpAuthType;
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthKind {
    None,
    ApiKey,
    BearerToken,
    Basic,
    Authorization,
    Oauth2,
    Token,
    Oauth2TokenExchange,
}

impl AuthKind {
    pub(crate) const fn mode(self) -> McpAuthType {
        match self {
            Self::None => McpAuthType::None,
            Self::ApiKey => McpAuthType::ApiKey,
            Self::BearerToken => McpAuthType::BearerToken,
            Self::Basic => McpAuthType::Basic,
            Self::Authorization => McpAuthType::Authorization,
            Self::Oauth2 => McpAuthType::Oauth2,
            Self::Token => McpAuthType::Token,
            Self::Oauth2TokenExchange => McpAuthType::Oauth2TokenExchange,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolNameInput {
    pub(crate) raw: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BaseUrlInput {
    pub(crate) spec: Value,
    #[serde(default)]
    pub(crate) spec_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResolveParamsInput {
    pub(crate) spec_operation: Value,
    pub(crate) path_item: Value,
    pub(crate) components: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OperationInput {
    pub(crate) spec_operation: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MergeHeadersInput {
    #[serde(default)]
    pub(crate) static_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) auth_header: Option<String>,
    #[serde(default)]
    pub(crate) resolved_auth_headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CallInput {
    pub(crate) path: String,
    pub(crate) method: String,
    pub(crate) spec_operation: Value,
    pub(crate) base_url: String,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) server_label: Option<String>,
    #[serde(default)]
    pub(crate) relays_upstream_auth: bool,
    #[serde(default)]
    pub(crate) auth_type: Option<AuthKind>,
    #[serde(default)]
    pub(crate) upstream_token_header: Option<String>,
    pub(crate) arguments: Value,
    #[serde(default)]
    pub(crate) auth_header: Option<String>,
    #[serde(default)]
    pub(crate) extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) resolved_auth_headers: BTreeMap<String, String>,
    pub(crate) reply: Reply,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    ToolName(ToolNameInput),
    BaseUrl(BaseUrlInput),
    ResolveParams(ResolveParamsInput),
    ExtractParams(OperationInput),
    #[serde(rename = "input_schema")]
    Schema(OperationInput),
    MergeHeaders(MergeHeadersInput),
    Call(Box<CallInput>),
}
