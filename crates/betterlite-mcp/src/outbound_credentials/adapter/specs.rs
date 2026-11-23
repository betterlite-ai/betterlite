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
use crate::credential_contracts::errors::CredResult;
use crate::credential_contracts::header_carrier::HeaderCarrier;
use crate::credential_contracts::server_spec::ServerSpec;
use crate::credentials::{strip_auth_scheme, to_basic_credentials};
use crate::oauth_resource::resource_identity::{UpstreamServer, resolve_upstream_resource};
use crate::server_record::{MCPServer, SecretCredential};

use super::{credential_header, present, token_endpoint_method};

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
    let configured = present(server.fields.upstream_token_header.as_deref());
    let header = configured.unwrap_or_else(|| default_header.to_owned());
    let config = ApiKeyConfig {
        header: HeaderCarrier::new(&header, value_prefix)?,
        key_source: ApiKeySource::Shared(SharedKey {
            value: SecretString::from(value),
        }),
    };
    Ok(Some(spec(server, resource, AuthConfig::ApiKey(config))))
}

pub(super) fn oauth2(server: &MCPServer, resource: &str) -> CredResult<Option<ServerSpec>> {
    if server.has_client_credentials() {
        return client_credentials(server, resource);
    }
    if server.needs_user_oauth_token() && !server.fields.delegate_auth_to_upstream {
        let config = AuthorizationCodeConfig {
            header: credential_header(server)?,
            scopes: Vec::new(),
            client_id: None,
            client_secret: None,
            authorization_url: None,
            token_url: None,
        };
        return Ok(Some(spec(
            server,
            resource,
            AuthConfig::AuthorizationCode(config),
        )));
    }
    Ok(None)
}

pub(super) fn client_credentials(
    server: &MCPServer,
    resource: &str,
) -> CredResult<Option<ServerSpec>> {
    let upstream = UpstreamServer {
        upstream_resource: server.fields.upstream_resource.as_deref(),
        url: server.fields.url.as_deref(),
        server_id: &server.fields.server_id,
    };
    let config = ClientCredentialsConfig {
        header: credential_header(server)?,
        client_id: server.fields.client_id.clone(),
        client_secret: client_secret(server),
        token_url: server.effective_token_url().map(str::to_owned),
        scopes: server.fields.scopes.clone().unwrap_or_default(),
        audience: server.fields.audience.clone(),
        upstream_resource: resolve_upstream_resource(&upstream),
        token_endpoint_auth_method: token_endpoint_method(server.fields.token_endpoint_auth_method),
    };
    Ok(Some(spec(
        server,
        resource,
        AuthConfig::ClientCredentials(config),
    )))
}

pub(super) fn token_exchange(server: &MCPServer, resource: &str) -> CredResult<Option<ServerSpec>> {
    let configured_endpoint = present(server.fields.token_exchange_endpoint.as_deref());
    let endpoint = configured_endpoint.or_else(|| server.effective_token_url().map(str::to_owned));
    let profile = if server.fields.token_exchange_profile == "entra_obo" {
        TokenExchangeProfile::EntraObo
    } else {
        TokenExchangeProfile::Rfc8693
    };
    let subject_token_type = present(Some(server.fields.subject_token_type.as_str()))
        .unwrap_or_else(|| DEFAULT_SUBJECT_TOKEN_TYPE.to_owned());
    let config = TokenExchangeConfig {
        header: credential_header(server)?,
        profile,
        subject_token_type,
        token_exchange_endpoint: endpoint,
        audience: server.fields.audience.clone(),
        client_id: server.fields.client_id.clone(),
        client_secret: client_secret(server),
        token_endpoint_auth_method: token_endpoint_method(server.fields.token_endpoint_auth_method),
        scopes: server.fields.scopes.clone().unwrap_or_default(),
    };
    Ok(Some(spec(
        server,
        resource,
        AuthConfig::TokenExchange(config),
    )))
}

pub(super) fn id_jag(server: &MCPServer, resource: &str) -> CredResult<Option<ServerSpec>> {
    let org_endpoint = present(server.fields.token_exchange_endpoint.as_deref());
    let resource_endpoint = present(server.fields.id_jag_resource_token_endpoint.as_deref());
    let client_id = present(server.fields.client_id.as_deref());
    let client_auth = id_jag_client_auth(server);
    let (Some(org_endpoint), Some(resource_endpoint), Some(client_id), Some(client_auth)) =
        (org_endpoint, resource_endpoint, client_id, client_auth)
    else {
        return Ok(None);
    };
    let config = IdJagConfig {
        header: credential_header(server)?,
        org_token_endpoint: org_endpoint,
        resource_token_endpoint: resource_endpoint,
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
    if let Some(private_key) = private_key {
        return Some(ClientAuth::PrivateKeyJwt(PrivateKeyJwtAuth {
            private_key: SecretString::from(private_key),
            key_id: server.fields.client_private_key_id.clone(),
            signing_alg: server.fields.client_assertion_signing_alg.clone(),
        }));
    }
    let secret = client_secret(server)?;
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

fn client_secret(server: &MCPServer) -> Option<SecretString> {
    let text = secret_text(server.fields.client_secret.as_ref())?;
    Some(SecretString::from(text))
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
