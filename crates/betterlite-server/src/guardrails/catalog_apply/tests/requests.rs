use axum::body::Body as HttpBody;
use axum::http::Request;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};

use crate::auth::auth_identity::AuthContext;

pub(super) fn caller() -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: "virtual-key".to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: Vec::new(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

pub(super) fn request(
    method: &str,
    uri: &str,
    body: &str,
    authenticated: bool,
) -> Request<HttpBody> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(HttpBody::from(body.to_owned()))
        .unwrap();
    if authenticated {
        request.extensions_mut().insert(caller());
    }
    request
}
