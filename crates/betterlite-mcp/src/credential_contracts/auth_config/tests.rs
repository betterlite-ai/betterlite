#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;

use super::AuthConfig;
use super::aws::{AwsCredentialSource, AwsSigV4Config, DEFAULT_AWS_SIGV4_SERVICE};
use super::id_jag::{ClientAuth, ClientSecretAuth, ID_JAG_SUBJECT_TOKEN_TYPE, IdJagConfig};
use super::key_source::{ApiKeyConfig, ApiKeySource, SharedKey};
use super::oauth::{AuthorizationCodeConfig, ClientCredentialsConfig, TokenExchangeConfig};
use super::simple::{NoneConfig, PassthroughConfig};
use crate::credential_contracts::auth_spec::AuthSpecKind;
use crate::credential_contracts::header_carrier::HeaderCarrier;

fn id_jag_config() -> IdJagConfig {
    IdJagConfig {
        header: HeaderCarrier::default(),
        org_token_endpoint: "https://org.example.test/token".to_owned(),
        resource_token_endpoint: "https://resource.example.test/token".to_owned(),
        client_id: "gateway-client".to_owned(),
        client_auth: ClientAuth::ClientSecret(ClientSecretAuth {
            client_secret: SecretString::from("secret".to_owned()),
        }),
        subject_token_type: ID_JAG_SUBJECT_TOKEN_TYPE.to_owned(),
        audience: None,
        resource: None,
        scopes: Vec::new(),
    }
}

#[rstest]
#[case::authorization_code(
    AuthConfig::AuthorizationCode(AuthorizationCodeConfig::default()),
    AuthSpecKind::AuthorizationCode
)]
#[case::client_credentials(
    AuthConfig::ClientCredentials(ClientCredentialsConfig::default()),
    AuthSpecKind::ClientCredentials
)]
#[case::token_exchange(
    AuthConfig::TokenExchange(TokenExchangeConfig::default()),
    AuthSpecKind::TokenExchange
)]
#[case::id_jag(AuthConfig::IdJag(id_jag_config()), AuthSpecKind::IdJag)]
#[case::api_key(
    AuthConfig::ApiKey(ApiKeyConfig {
        header: HeaderCarrier::default(),
        key_source: ApiKeySource::Shared(SharedKey {
            value: SecretString::from("key".to_owned()),
        }),
    }),
    AuthSpecKind::ApiKey
)]
#[case::passthrough(AuthConfig::Passthrough(PassthroughConfig), AuthSpecKind::Passthrough)]
#[case::none(AuthConfig::None(NoneConfig), AuthSpecKind::None)]
#[case::aws_sigv4(
    AuthConfig::AwsSigv4(AwsSigV4Config {
        region: "us-east-1".to_owned(),
        service: DEFAULT_AWS_SIGV4_SERVICE.to_owned(),
        credentials: AwsCredentialSource::default(),
    }),
    AuthSpecKind::AwsSigv4
)]
fn each_auth_config_kind(#[case] config: AuthConfig, #[case] expected: AuthSpecKind) {
    assert_eq!(config.kind(), expected);
}

// kind() matches one arm per variant with no wildcard, so a cross-mode config fails to compile.
fn as_aws_sigv4(config: AuthConfig) -> Option<AwsSigV4Config> {
    match config {
        AuthConfig::AwsSigv4(inner) => Some(inner),
        AuthConfig::AuthorizationCode(_)
        | AuthConfig::ClientCredentials(_)
        | AuthConfig::TokenExchange(_)
        | AuthConfig::IdJag(_)
        | AuthConfig::ApiKey(_)
        | AuthConfig::Passthrough(_)
        | AuthConfig::None(_) => None,
    }
}

fn as_id_jag(config: AuthConfig) -> Option<IdJagConfig> {
    match config {
        AuthConfig::IdJag(inner) => Some(inner),
        AuthConfig::AuthorizationCode(_)
        | AuthConfig::ClientCredentials(_)
        | AuthConfig::TokenExchange(_)
        | AuthConfig::ApiKey(_)
        | AuthConfig::Passthrough(_)
        | AuthConfig::None(_)
        | AuthConfig::AwsSigv4(_) => None,
    }
}

#[rstest]
fn illegal_state_unrepresentable() {
    let aws = AuthConfig::AwsSigv4(AwsSigV4Config {
        region: "eu-west-1".to_owned(),
        service: DEFAULT_AWS_SIGV4_SERVICE.to_owned(),
        credentials: AwsCredentialSource::default(),
    });
    let config = as_aws_sigv4(aws).expect("constructed as AwsSigv4");
    assert_eq!(config.region, "eu-west-1");
    assert_eq!(config.service, DEFAULT_AWS_SIGV4_SERVICE);

    let id_jag = AuthConfig::IdJag(id_jag_config());
    let jag_config = as_id_jag(id_jag).expect("constructed as IdJag");
    assert_eq!(jag_config.client_id, "gateway-client");
}
