pub mod aws;
pub mod id_jag;
pub mod key_source;
pub mod oauth;
pub mod simple;
mod tests;

use super::auth_spec::AuthSpecKind;
use aws::AwsSigV4Config;
use id_jag::IdJagConfig;
use key_source::ApiKeyConfig;
use oauth::{AuthorizationCodeConfig, ClientCredentialsConfig, TokenExchangeConfig};
use simple::{NoneConfig, PassthroughConfig};

#[derive(Debug, Clone)]
pub enum AuthConfig {
    AuthorizationCode(AuthorizationCodeConfig),
    ClientCredentials(ClientCredentialsConfig),
    TokenExchange(TokenExchangeConfig),
    IdJag(IdJagConfig),
    ApiKey(ApiKeyConfig),
    Passthrough(PassthroughConfig),
    None(NoneConfig),
    AwsSigv4(AwsSigV4Config),
}

impl AuthConfig {
    #[must_use]
    pub const fn kind(&self) -> AuthSpecKind {
        match self {
            Self::AuthorizationCode(_) => AuthSpecKind::AuthorizationCode,
            Self::ClientCredentials(_) => AuthSpecKind::ClientCredentials,
            Self::TokenExchange(_) => AuthSpecKind::TokenExchange,
            Self::IdJag(_) => AuthSpecKind::IdJag,
            Self::ApiKey(_) => AuthSpecKind::ApiKey,
            Self::Passthrough(_) => AuthSpecKind::Passthrough,
            Self::None(_) => AuthSpecKind::None,
            Self::AwsSigv4(_) => AuthSpecKind::AwsSigv4,
        }
    }
}
