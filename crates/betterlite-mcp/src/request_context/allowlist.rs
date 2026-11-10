use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use http::HeaderMap;
use http::header::HeaderValue;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

pub const MCP_ALLOWED_CLIENTS_SETTING: &str = "mcp_allowed_clients";
pub const MCP_CLIENT_ID_HEADER_SETTING: &str = "mcp_client_id_header";
pub const CLIENT_ID_JWT_FIELD_SETTING: &str = "mcp_client_id_jwt_field";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct McpAllowedClient {
    pub alias: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct JwtAuthSettings {
    pub mcp_client_id_jwt_field: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct McpClientAllowlistSettings {
    pub mcp_allowed_clients: Option<Vec<McpAllowedClient>>,
    pub mcp_client_id_header: Option<String>,
    pub litellm_jwtauth: Option<JwtAuthSettings>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct McpClientAllowlist {
    pub aliases_by_value: BTreeMap<String, String>,
    pub jwt_field: Option<String>,
    pub header: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentitySource {
    Jwt,
    Header,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpClientIdentity {
    pub client_id: String,
    pub source: IdentitySource,
    pub source_name: String,
}

impl McpClientIdentity {
    #[must_use]
    pub fn description(&self) -> String {
        let source = match self.source {
            IdentitySource::Jwt => "JWT claim",
            IdentitySource::Header => "header",
        };
        format!(
            "'{}' (from {source} '{}')",
            self.client_id, self.source_name
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ForbiddenError {
    #[serde(rename = "Forbidden")]
    Forbidden,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct McpClientForbiddenBody {
    pub error: ForbiddenError,
    pub details: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpClientRejection {
    pub details: String,
}

impl McpClientRejection {
    #[must_use]
    pub fn response_body(&self) -> McpClientForbiddenBody {
        McpClientForbiddenBody {
            error: ForbiddenError::Forbidden,
            details: self.details.clone(),
        }
    }
}

pub fn load_mcp_client_allowlist(
    settings: &McpClientAllowlistSettings,
) -> Option<McpClientAllowlist> {
    let allowed_clients = settings.mcp_allowed_clients.as_ref()?;
    let header = settings
        .mcp_client_id_header
        .as_deref()
        .filter(|name| !name.is_empty())
        .map(str::to_lowercase);
    let jwt_field = settings
        .litellm_jwtauth
        .as_ref()
        .and_then(|jwt_auth| jwt_auth.mcp_client_id_jwt_field.as_deref())
        .filter(|field| !field.is_empty())
        .map(str::to_owned);
    let malformed = allowed_clients
        .iter()
        .any(|client| client.alias.is_empty() || client.value.is_empty());
    if malformed {
        warn!(
            "{MCP_ALLOWED_CLIENTS_SETTING} has an entry with a blank alias or value; rejecting every MCP client until it is fixed"
        );
    }
    Some(McpClientAllowlist {
        aliases_by_value: if malformed {
            BTreeMap::new()
        } else {
            allowed_clients
                .iter()
                .map(|client| (client.value.clone(), client.alias.clone()))
                .collect()
        },
        jwt_field,
        header,
    })
}

pub fn resolve_mcp_client_identity(
    allowlist: &McpClientAllowlist,
    jwt_claims: Option<&Payload>,
    headers: &HeaderMap,
) -> Result<McpClientIdentity, McpClientRejection> {
    if let (Some(claims), Some(field)) = (jwt_claims, allowlist.jwt_field.as_deref()) {
        return match claim_value(claims, field) {
            Some(JsonData::String(client_id)) if !client_id.is_empty() => Ok(McpClientIdentity {
                client_id: client_id.clone(),
                source: IdentitySource::Jwt,
                source_name: field.to_owned(),
            }),
            Some(_) | None => Err(unidentified_rejection(&format!(
                "The JWT presented has no '{field}' claim naming the client application."
            ))),
        };
    }
    let Some(header) = allowlist.header.as_deref() else {
        return Err(unidentified_rejection(&format!(
            "No client identity source is configured for this request; set litellm_jwtauth.{CLIENT_ID_JWT_FIELD_SETTING} for JWT callers or {MCP_CLIENT_ID_HEADER_SETTING} in general_settings."
        )));
    };
    match headers.get(header).map(latin1) {
        Some(client_id) if !client_id.is_empty() => Ok(McpClientIdentity {
            client_id,
            source: IdentitySource::Header,
            source_name: header.to_owned(),
        }),
        Some(_) | None => Err(unidentified_rejection(&format!(
            "The request has no '{header}' header naming the client application."
        ))),
    }
}

pub fn check_mcp_client_allowed(
    allowlist: Option<&McpClientAllowlist>,
    jwt_claims: Option<&Payload>,
    headers: &HeaderMap,
) -> Result<(), McpClientRejection> {
    let Some(enforced) = allowlist else {
        return Ok(());
    };
    let identity = resolve_mcp_client_identity(enforced, jwt_claims, headers)?;
    let Some(alias) = enforced.aliases_by_value.get(&identity.client_id) else {
        return Err(McpClientRejection {
            details: format!(
                "MCP client {} is not listed in this gateway's {MCP_ALLOWED_CLIENTS_SETTING}.",
                identity.description()
            ),
        });
    };
    let description = identity.description();
    debug!("Admitted MCP client '{alias}' identified as {description}");
    Ok(())
}

fn unidentified_rejection(reason: &str) -> McpClientRejection {
    McpClientRejection {
        details: format!(
            "{reason} This gateway only admits client applications listed in {MCP_ALLOWED_CLIENTS_SETTING}."
        ),
    }
}

fn claim_value<'claims>(claims: &'claims Payload, key_path: &str) -> Option<&'claims JsonData> {
    if key_path.is_empty() {
        return None;
    }
    let path = key_path.strip_prefix("metadata.").unwrap_or(key_path);
    let parts = split_key_path(path);
    let mut current = claims.0.get(parts.first()?.as_str());
    for part in parts.iter().skip(1) {
        let Some(JsonData::Object(fields)) = current else {
            return None;
        };
        current = fields.get(part.as_str());
    }
    current
}

fn split_key_path(path: &str) -> Vec<String> {
    path.replace("\\.", "\u{0}")
        .split('.')
        .map(|part| part.replace('\u{0}', "."))
        .collect()
}

fn latin1(value: &HeaderValue) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| char::from(*byte))
        .collect()
}

mod tests;
