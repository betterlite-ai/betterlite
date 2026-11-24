use betterlite_db::mcp_stored_contracts::MCPAuth;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credential_contracts::auth_config::AuthConfig;
use crate::credential_contracts::auth_config::id_jag::{
    ClientAuth, ClientSecretAuth, ID_JAG_SUBJECT_TOKEN_TYPE, IdJagConfig, PrivateKeyJwtAuth,
};
use crate::credential_contracts::auth_config::key_source::{ApiKeyConfig, ApiKeySource, SharedKey};
use crate::credential_contracts::auth_config::oauth::{
    AuthorizationCodeConfig, ClientCredentialsConfig, DEFAULT_SUBJECT_TOKEN_TYPE,
    TokenExchangeConfig, TokenExchangeProfile,
};
use crate::credential_contracts::auth_config::simple::{NoneConfig, PassthroughConfig};
use crate::credential_contracts::errors::CredResult;
use crate::credential_contracts::header_carrier::HeaderCarrier;
use crate::credential_contracts::server_spec::ServerSpec;
use crate::credentials::{strip_auth_scheme, to_basic_credentials};
use crate::oauth_resource::resource_identity::{UpstreamServer, resolve_upstream_resource};
use crate::server_record::{MCPServer, SecretCredential};

use super::{VALUE_PREFIX_BEARER, token_endpoint_method, token_header};

#[derive(Clone, Copy)]
pub(super) enum SpecMode {
    Oauth2,
    TokenExchange,
    IdJag,
}

pub(super) fn shared_key(
    server: &MCPServer,
    resource: &str,
    default_header: &str,
    value_prefix: &str,
    encode: bool,
) -> CredResult<Option<ServerSpec>> {
    let Some(token) = secret_text(server.fields.authentication_token.as_ref()) else {
        return Ok(None);
    };
    let value = if encode {
        to_basic_credentials(&token)
    } else if value_prefix.is_empty() {
        token
    } else {
        strip_auth_scheme(&token, value_prefix)
    };
    let header = server
        .fields
        .upstream_token_header
        .as_deref()
        .filter(|text| !text.is_empty())
        .map_or_else(|| default_header.to_owned(), str::to_owned);
    let config = ApiKeyConfig {
        header: HeaderCarrier::new(&header, value_prefix)?,
        key_source: ApiKeySource::Shared(SharedKey {
            value: SecretString::from(value),
        }),
    };
    Ok(Some(spec(server, resource, AuthConfig::ApiKey(config))))
}

pub(super) fn static_shared_key(
    server: &MCPServer,
    resource: &str,
    auth_type: MCPAuth,
) -> CredResult<Option<ServerSpec>> {
    let (header, value_prefix, encode) = match auth_type {
        MCPAuth::ApiKey => ("X-API-Key", "", false),
        MCPAuth::BearerToken => ("Authorization", VALUE_PREFIX_BEARER, false),
        MCPAuth::Token => ("Authorization", "token", false),
        MCPAuth::Authorization => ("Authorization", "", false),
        MCPAuth::Basic => ("Authorization", "Basic", true),
        MCPAuth::None
        | MCPAuth::Oauth2
        | MCPAuth::AwsSigv4
        | MCPAuth::Oauth2TokenExchange
        | MCPAuth::Oauth2IdJag
        | MCPAuth::TruePassthrough
        | MCPAuth::OauthDelegate => return Ok(None),
    };
    shared_key(server, resource, header, value_prefix, encode)
}

pub(super) fn passthrough(server: &MCPServer, resource: String) -> ServerSpec {
    ServerSpec {
        server_id: server.fields.server_id.clone(),
        resource,
        config: AuthConfig::Passthrough(PassthroughConfig),
    }
}

pub(super) fn none_spec(server: &MCPServer, resource: String) -> Option<ServerSpec> {
    if server.is_oauth_passthrough() {
        return None;
    }
    Some(ServerSpec {
        server_id: server.fields.server_id.clone(),
        resource,
        config: AuthConfig::None(NoneConfig),
    })
}

pub(super) fn build(
    server: &MCPServer,
    resource: &str,
    mode: SpecMode,
) -> CredResult<Option<ServerSpec>> {
    match mode {
        SpecMode::Oauth2 => oauth2(server, resource),
        SpecMode::TokenExchange => oauth_spec(server, resource, Grant::TokenExchange),
        SpecMode::IdJag => id_jag(server, resource),
    }
}

fn oauth2(server: &MCPServer, resource: &str) -> CredResult<Option<ServerSpec>> {
    if server.has_client_credentials() {
        return oauth_spec(server, resource, Grant::ClientCredentials);
    }
    if server.needs_user_oauth_token() && !server.fields.delegate_auth_to_upstream {
        return authorization_code(server, resource);
    }
    Ok(None)
}

#[derive(Clone, Copy)]
enum Grant {
    ClientCredentials,
    TokenExchange,
}

// Both OAuth grants carry the same client registration, so one match builds either.
fn oauth_spec(server: &MCPServer, resource: &str, grant: Grant) -> CredResult<Option<ServerSpec>> {
    let config = match grant {
        Grant::ClientCredentials => {
            let upstream = UpstreamServer {
                upstream_resource: server.fields.upstream_resource.as_deref(),
                url: server.fields.url.as_deref(),
                server_id: &server.fields.server_id,
            };
            AuthConfig::ClientCredentials(ClientCredentialsConfig {
                header: HeaderCarrier::new(&token_header(server), VALUE_PREFIX_BEARER)?,
                client_id: server.fields.client_id.clone(),
                client_secret: secret_text(server.fields.client_secret.as_ref())
                    .map(SecretString::from),
                token_url: server.effective_token_url().map(str::to_owned),
                scopes: server.fields.scopes.clone().unwrap_or_default(),
                audience: server.fields.audience.clone(),
                upstream_resource: resolve_upstream_resource(&upstream),
                token_endpoint_auth_method: token_endpoint_method(
                    server.fields.token_endpoint_auth_method,
                ),
            })
        }
        Grant::TokenExchange => AuthConfig::TokenExchange(TokenExchangeConfig {
            header: HeaderCarrier::new(&token_header(server), VALUE_PREFIX_BEARER)?,
            profile: token_exchange_profile(server),
            subject_token_type: exchange_subject_token_type(server),
            token_exchange_endpoint: exchange_endpoint(server),
            audience: server.fields.audience.clone(),
            client_id: server.fields.client_id.clone(),
            client_secret: secret_text(server.fields.client_secret.as_ref())
                .map(SecretString::from),
            token_endpoint_auth_method: token_endpoint_method(
                server.fields.token_endpoint_auth_method,
            ),
            scopes: server.fields.scopes.clone().unwrap_or_default(),
        }),
    };
    Ok(Some(spec(server, resource, config)))
}

fn authorization_code(server: &MCPServer, resource: &str) -> CredResult<Option<ServerSpec>> {
    let config = AuthorizationCodeConfig {
        header: HeaderCarrier::new(&token_header(server), VALUE_PREFIX_BEARER)?,
        scopes: Vec::new(),
        client_id: None,
        client_secret: None,
        authorization_url: None,
        token_url: None,
    };
    Ok(Some(spec(
        server,
        resource,
        AuthConfig::AuthorizationCode(config),
    )))
}

fn exchange_endpoint(server: &MCPServer) -> Option<String> {
    non_empty_text(server.fields.token_exchange_endpoint.as_deref())
        .or_else(|| server.effective_token_url().map(str::to_owned))
}

fn non_empty_text(value: Option<&str>) -> Option<String> {
    let text = value?;
    if text.is_empty() {
        return None;
    }
    Some(text.to_owned())
}

fn token_exchange_profile(server: &MCPServer) -> TokenExchangeProfile {
    if server.fields.token_exchange_profile == "entra_obo" {
        TokenExchangeProfile::EntraObo
    } else {
        TokenExchangeProfile::Rfc8693
    }
}

fn exchange_subject_token_type(server: &MCPServer) -> String {
    let configured = server.fields.subject_token_type.as_str();
    if configured.is_empty() {
        return DEFAULT_SUBJECT_TOKEN_TYPE.to_owned();
    }
    configured.to_owned()
}

fn id_jag(server: &MCPServer, resource: &str) -> CredResult<Option<ServerSpec>> {
    let Some(org_token_endpoint) = non_empty_text(server.fields.token_exchange_endpoint.as_deref())
    else {
        return Ok(None);
    };
    let Some(resource_token_endpoint) =
        non_empty_text(server.fields.id_jag_resource_token_endpoint.as_deref())
    else {
        return Ok(None);
    };
    let Some(client_id) = non_empty_text(server.fields.client_id.as_deref()) else {
        return Ok(None);
    };
    let Some(client_auth) = id_jag_client_auth(server) else {
        return Ok(None);
    };
    let config = IdJagConfig {
        header: HeaderCarrier::new(&token_header(server), VALUE_PREFIX_BEARER)?,
        org_token_endpoint,
        resource_token_endpoint,
        client_id,
        client_auth,
        subject_token_type: id_jag_subject_token_type(server),
        audience: server.fields.audience.clone(),
        resource: server.fields.id_jag_resource.clone(),
        scopes: server.fields.scopes.clone().unwrap_or_default(),
    };
    Ok(Some(spec(server, resource, AuthConfig::IdJag(config))))
}

fn id_jag_client_auth(server: &MCPServer) -> Option<ClientAuth> {
    let private_key = secret_text(server.fields.client_private_key.as_ref());
    if let Some(key) = private_key {
        return Some(ClientAuth::PrivateKeyJwt(PrivateKeyJwtAuth {
            private_key: SecretString::from(key),
            key_id: server.fields.client_private_key_id.clone(),
            signing_alg: server.fields.client_assertion_signing_alg.clone(),
        }));
    }
    let secret = SecretString::from(secret_text(server.fields.client_secret.as_ref())?);
    Some(ClientAuth::ClientSecret(ClientSecretAuth {
        client_secret: secret,
    }))
}

// ID-JAG asserts the user's id_token, so the exchange default maps over; an explicit value (a SAML2 assertion type) is kept verbatim.
fn id_jag_subject_token_type(server: &MCPServer) -> String {
    let configured = server.fields.subject_token_type.as_str();
    if !configured.is_empty() && configured != DEFAULT_SUBJECT_TOKEN_TYPE {
        return configured.to_owned();
    }
    ID_JAG_SUBJECT_TOKEN_TYPE.to_owned()
}

fn secret_text(secret: Option<&SecretCredential>) -> Option<String> {
    let value = secret?.0.0.expose_secret();
    if value.is_empty() {
        return None;
    }
    Some(value.to_owned())
}

fn spec(server: &MCPServer, resource: &str, config: AuthConfig) -> ServerSpec {
    let server_id = server.fields.server_id.clone();
    ServerSpec {
        server_id,
        resource: resource.to_owned(),
        config,
    }
}
