use std::sync::Arc;

use crate::credential_contracts::auth_config::AuthConfig;
use crate::credential_contracts::auth_spec::AuthResolution;
use crate::credential_contracts::errors::CredResult;
use crate::credential_contracts::server_spec::{ServerSpec, Subject};
use crate::credentials::ResolvedAuth;

use super::UpstreamCredentialProvider;

#[derive(Clone)]
pub struct ResolvedCredential {
    pub auth: Arc<dyn ResolvedAuth>,
    pub source: AuthResolution,
}

pub async fn resolve_with_source(
    provider: &UpstreamCredentialProvider,
    subject: &Subject,
    server: &ServerSpec,
) -> CredResult<ResolvedCredential> {
    let auth = provider.resolve_credentials(subject, server).await?;
    let source = auth_source(subject, server);
    Ok(ResolvedCredential { auth, source })
}

// A passthrough server that forwarded no token resolved nothing, which is the same no-auth answer the none mode gives.
const fn auth_source(subject: &Subject, server: &ServerSpec) -> AuthResolution {
    match &server.config {
        AuthConfig::None(_config) => AuthResolution::NoAuth,
        AuthConfig::Passthrough(_config) => {
            if subject.inbound_token.is_some() {
                AuthResolution::Oauth2Passthrough
            } else {
                AuthResolution::NoAuth
            }
        }
        AuthConfig::ApiKey(_config) => AuthResolution::StaticToken,
        AuthConfig::ClientCredentials(_config) => AuthResolution::ClientCredentials,
        AuthConfig::TokenExchange(_config) => AuthResolution::TokenExchange,
        AuthConfig::IdJag(_config) => AuthResolution::IdJag,
        AuthConfig::AuthorizationCode(_config) => AuthResolution::StoredUserToken,
        AuthConfig::AwsSigv4(_config) => AuthResolution::AwsSigv4,
    }
}
