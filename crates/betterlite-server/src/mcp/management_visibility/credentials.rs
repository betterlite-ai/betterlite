use betterlite_db::mcp_server_encoding::request::Maybe;
use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use betterlite_db::mcp_stored_contracts::{MCPCredentials, MCPTokenEndpointAuthMethod};
use betterlite_mcp::server_record::{SecretCredential, ServerFields};
use secrecy::ExposeSecret as _;

// LiteLLM reads a credential value as supplied only when it is truthy, so a null and an empty value are both absent
pub(super) fn secret_supplied(value: Option<&Option<StoredSecret>>) -> bool {
    let Some(Some(secret)) = value else {
        return false;
    };
    !secret.0.expose_secret().is_empty()
}

pub(super) const fn text_supplied(value: Option<&Option<String>>) -> bool {
    let Some(Some(text)) = value else {
        return false;
    };
    !text.is_empty()
}

pub(super) const fn texts_supplied(value: Option<&Option<Vec<String>>>) -> bool {
    let Some(Some(texts)) = value else {
        return false;
    };
    !texts.is_empty()
}

pub(super) const fn method_supplied(value: Option<&Option<MCPTokenEndpointAuthMethod>>) -> bool {
    let Some(method) = value else {
        return false;
    };
    method.is_some()
}

// _inherit_credentials_from_existing_server's merge: the parent fills every field the payload left out
pub(super) fn inherited_credentials(
    parent: &ServerFields,
    payload: Option<&MCPCredentials>,
) -> Option<MCPCredentials> {
    let inherited = inherited_from_parent(parent);
    let Some(supplied) = payload else {
        return any_inherited(&inherited).then_some(inherited);
    };
    Some(with_supplied(supplied, inherited))
}

fn any_inherited(credentials: &MCPCredentials) -> bool {
    secret_supplied(credentials.auth_value.as_ref())
        || text_supplied(credentials.client_id.as_ref())
        || secret_supplied(credentials.client_secret.as_ref())
        || texts_supplied(credentials.scopes.as_ref())
        || secret_supplied(credentials.aws_access_key_id.as_ref())
        || secret_supplied(credentials.aws_secret_access_key.as_ref())
        || secret_supplied(credentials.aws_session_token.as_ref())
        || text_supplied(credentials.aws_region_name.as_ref())
        || text_supplied(credentials.aws_service_name.as_ref())
        || text_supplied(credentials.upstream_resource.as_ref())
        || text_supplied(credentials.upstream_token_header.as_ref())
}

fn inherited_from_parent(parent: &ServerFields) -> MCPCredentials {
    MCPCredentials {
        auth_value: secret_inherited(parent.authentication_token.as_ref()),
        client_id: text_inherited(parent.client_id.as_deref()),
        client_secret: secret_inherited(parent.client_secret.as_ref()),
        scopes: texts_inherited(parent.scopes.as_deref()),
        aws_access_key_id: secret_inherited(parent.aws_access_key_id.as_ref()),
        aws_secret_access_key: secret_inherited(parent.aws_secret_access_key.as_ref()),
        aws_session_token: secret_inherited(parent.aws_session_token.as_ref()),
        aws_region_name: text_inherited(parent.aws_region_name.as_deref()),
        aws_service_name: text_inherited(parent.aws_service_name.as_deref()),
        aws_role_name: None,
        aws_session_name: None,
        audience: None,
        token_exchange_endpoint: None,
        subject_token_type: None,
        id_jag_resource_token_endpoint: None,
        id_jag_resource: None,
        upstream_resource: text_inherited(parent.upstream_resource.as_deref()),
        upstream_token_header: text_inherited(parent.upstream_token_header.as_deref()),
        client_private_key: None,
        client_private_key_id: None,
        client_assertion_signing_alg: None,
        token_endpoint_auth_method: None,
        redirect_uris: None,
        token_exchange_profile: None,
    }
}

// LiteLLM's getattr filter inherits a parent value only when it is truthy, so a null, an empty string and an empty list are all dropped
fn secret_inherited(parent: Option<&SecretCredential>) -> Maybe<StoredSecret> {
    parent
        .filter(|credential| !credential.0.0.expose_secret().is_empty())
        .map(|credential| Some(credential.0.clone()))
}

fn text_inherited(parent: Option<&str>) -> Maybe<String> {
    parent
        .filter(|text| !text.is_empty())
        .map(|text| Some(text.to_owned()))
}

fn texts_inherited(parent: Option<&[String]>) -> Maybe<Vec<String>> {
    parent
        .filter(|texts| !texts.is_empty())
        .map(|texts| Some(texts.to_vec()))
}

// a field the payload names wins even when it is null or empty, exactly like the dict spread LiteLLM merges with
fn with_supplied(supplied: &MCPCredentials, inherited: MCPCredentials) -> MCPCredentials {
    let mut merged = supplied.clone();
    if merged.auth_value.is_none() {
        merged.auth_value = inherited.auth_value;
    }
    if merged.client_id.is_none() {
        merged.client_id = inherited.client_id;
    }
    if merged.client_secret.is_none() {
        merged.client_secret = inherited.client_secret;
    }
    if merged.scopes.is_none() {
        merged.scopes = inherited.scopes;
    }
    if merged.aws_access_key_id.is_none() {
        merged.aws_access_key_id = inherited.aws_access_key_id;
    }
    if merged.aws_secret_access_key.is_none() {
        merged.aws_secret_access_key = inherited.aws_secret_access_key;
    }
    if merged.aws_session_token.is_none() {
        merged.aws_session_token = inherited.aws_session_token;
    }
    if merged.aws_region_name.is_none() {
        merged.aws_region_name = inherited.aws_region_name;
    }
    if merged.aws_service_name.is_none() {
        merged.aws_service_name = inherited.aws_service_name;
    }
    if merged.upstream_resource.is_none() {
        merged.upstream_resource = inherited.upstream_resource;
    }
    if merged.upstream_token_header.is_none() {
        merged.upstream_token_header = inherited.upstream_token_header;
    }
    merged
}
