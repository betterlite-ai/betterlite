pub mod challenge;
pub mod errors;
pub mod prepare;
mod specs;

use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPTokenEndpointAuthMethod};
use secrecy::SecretString;

use crate::credential_contracts::errors::CredResult;
use crate::credential_contracts::header_carrier::DEFAULT_CREDENTIAL_HEADER;
use crate::credential_contracts::server_spec::{ServerSpec, Subject};
use crate::credential_contracts::token_endpoint_auth::TokenEndpointAuthMethod;
use crate::credentials::McpAuthType;
use crate::server_record::MCPServer;

const VALUE_PREFIX_BEARER: &str = "Bearer";

#[derive(Debug, Clone, Default)]
pub struct UserPrincipal {
    pub org_id: Option<String>,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
}

#[must_use]
pub fn to_subject(principal: Option<&UserPrincipal>, subject_token: Option<&str>) -> Subject {
    let inbound_token = subject_token
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .map(SecretString::from);
    let Some(user) = principal else {
        return Subject {
            tenant_id: String::new(),
            subject_id: String::new(),
            inbound_token,
        };
    };
    let tenant_id = user
        .org_id
        .as_deref()
        .filter(|text| !text.is_empty())
        .or_else(|| user.team_id.as_deref().filter(|text| !text.is_empty()))
        .unwrap_or_default()
        .to_owned();
    let subject_id = user
        .user_id
        .as_deref()
        .filter(|text| !text.is_empty())
        .unwrap_or_default()
        .to_owned();
    Subject {
        tenant_id,
        subject_id,
        inbound_token,
    }
}

pub fn to_server_spec(server: &MCPServer) -> CredResult<Option<ServerSpec>> {
    let fields = &server.fields;
    if fields.is_byok && fields.auth_type != Some(MCPAuth::Oauth2TokenExchange) {
        return Ok(None);
    }
    let resource = resource_of(server);
    match fields.auth_type {
        Some(
            auth_type @ (MCPAuth::ApiKey
            | MCPAuth::BearerToken
            | MCPAuth::Token
            | MCPAuth::Authorization
            | MCPAuth::Basic),
        ) => specs::static_shared_key(server, &resource, auth_type),
        Some(MCPAuth::Oauth2) => specs::build(server, &resource, specs::SpecMode::Oauth2),
        Some(MCPAuth::Oauth2IdJag) => specs::build(server, &resource, specs::SpecMode::IdJag),
        Some(MCPAuth::Oauth2TokenExchange) => {
            specs::build(server, &resource, specs::SpecMode::TokenExchange)
        }
        Some(MCPAuth::AwsSigv4) => Ok(None),
        Some(MCPAuth::TruePassthrough | MCPAuth::OauthDelegate) => {
            Ok(Some(specs::passthrough(server, resource)))
        }
        None | Some(MCPAuth::None) => Ok(specs::none_spec(server, resource)),
    }
}

// The static-header family is the only set prepare_mcp_client checks; every other mode leaves the client untouched.
pub(super) const fn static_auth_type(auth_type: Option<MCPAuth>) -> Option<McpAuthType> {
    match auth_type {
        Some(MCPAuth::ApiKey) => Some(McpAuthType::ApiKey),
        Some(MCPAuth::BearerToken) => Some(McpAuthType::BearerToken),
        Some(MCPAuth::Basic) => Some(McpAuthType::Basic),
        Some(MCPAuth::Authorization) => Some(McpAuthType::Authorization),
        Some(MCPAuth::Token) => Some(McpAuthType::Token),
        Some(
            MCPAuth::None
            | MCPAuth::Oauth2
            | MCPAuth::AwsSigv4
            | MCPAuth::Oauth2TokenExchange
            | MCPAuth::Oauth2IdJag
            | MCPAuth::TruePassthrough
            | MCPAuth::OauthDelegate,
        )
        | None => None,
    }
}

pub(super) fn static_header_slots(server: &MCPServer) -> Vec<String> {
    let slots = server.fields.static_headers.as_ref();
    slots
        .map(|headers| headers.keys().cloned().collect())
        .unwrap_or_default()
}

pub(super) fn token_header(server: &MCPServer) -> String {
    server
        .fields
        .upstream_token_header
        .as_deref()
        .filter(|text| !text.is_empty())
        .map_or_else(|| DEFAULT_CREDENTIAL_HEADER.to_owned(), str::to_owned)
}

pub(super) const fn token_endpoint_method(
    raw: Option<MCPTokenEndpointAuthMethod>,
) -> Option<TokenEndpointAuthMethod> {
    match raw {
        Some(MCPTokenEndpointAuthMethod::ClientSecretBasic) => {
            Some(TokenEndpointAuthMethod::ClientSecretBasic)
        }
        Some(MCPTokenEndpointAuthMethod::ClientSecretPost) => {
            Some(TokenEndpointAuthMethod::ClientSecretPost)
        }
        None => None,
    }
}

fn resource_of(server: &MCPServer) -> String {
    server
        .fields
        .url
        .as_deref()
        .filter(|text| !text.is_empty())
        .map_or_else(|| server.fields.server_id.clone(), str::to_owned)
}
