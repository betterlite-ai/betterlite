use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::azure_default_chain::AzureCredentialType;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_client_secret::SecretResolver;
use crate::azure_default_chain::environment::ChainEnvironment;
use crate::azure_default_chain::grant::{
    CertificateRecord, ChainToken, ClientSecretRecord, Grant, GrantError, Grants, LinkError,
    ManagedIdentityRecord, UsernamePasswordRecord, WorkloadRecord,
};
use crate::credentials::grant_sources::GrantSources;
use crate::credentials::selection::AzureTokenError;
use crate::credentials::source::{
    CallableTokenProvider, CredentialSources as _, ProviderFailure, SourceFailure, TokenReply,
};
use crate::oidc_exchange::transport::{Clock, HttpReply, Transport};
use crate::oidc_exchange::{AzureOidcExchange, OidcExchangeInput};

pub(super) struct FakeGrants {
    construct_error: Option<&'static str>,
    link_error: Option<LinkError>,
}

pub(super) struct FakeGrant {
    name: &'static str,
    link_error: Option<LinkError>,
}

impl Grant for FakeGrant {
    fn token(&self) -> Result<ChainToken, LinkError> {
        if let Some(error) = &self.link_error {
            return Err(error.clone());
        }
        Ok(ChainToken {
            token: SecretString::from(format!("{}-token", self.name)),
            expires_on: None,
            token_type: None,
            refresh_on: None,
        })
    }
}

impl FakeGrants {
    fn grant(&self, name: &'static str) -> Result<FakeGrant, GrantError> {
        self.construct_error.map_or_else(
            || {
                Ok(FakeGrant {
                    name,
                    link_error: self.link_error.clone(),
                })
            },
            |message| {
                Err(GrantError {
                    credential: name,
                    message: message.to_owned(),
                })
            },
        )
    }
}

impl Grants for FakeGrants {
    type Grant<'grants>
        = FakeGrant
    where
        Self: 'grants;

    fn client_secret<'grants>(
        &'grants self,
        _record: &ClientSecretRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        self.grant("client_secret")
    }

    fn certificate<'grants>(
        &'grants self,
        _record: &CertificateRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        self.grant("certificate")
    }

    fn username_password<'grants>(
        &'grants self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        self.grant("username_password")
    }

    fn workload<'grants>(
        &'grants self,
        _record: &WorkloadRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        self.grant("workload")
    }

    fn managed_identity<'grants>(
        &'grants self,
        _record: &ManagedIdentityRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        self.grant("managed_identity")
    }
}

struct FixedTransport {
    status: u16,
    body: &'static str,
}

impl Transport for FixedTransport {
    fn post_form(&self, _url: &str, _form: &[(String, SecretString)]) -> io::Result<HttpReply> {
        Ok(HttpReply {
            status: self.status,
            body: SecretString::from(self.body),
        })
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> f64 {
        1_700_000_000.0
    }
}

struct FixedResolver;

impl SecretResolver for FixedResolver {
    fn get_secret_str(&self, _reference: &str) -> Option<SecretString> {
        Some(SecretString::from("assertion"))
    }
}

fn exchange(status: u16, body: &'static str) -> AzureOidcExchange {
    let settings = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .unwrap()
    .sections
    .azure_oidc_exchange;
    AzureOidcExchange::new(
        settings,
        BTreeMap::new(),
        Arc::new(FixedTransport { status, body }),
        Arc::new(FixedClock),
    )
    .unwrap()
}

fn oidc_input(identity: Option<&str>) -> OidcExchangeInput {
    OidcExchangeInput {
        azure_ad_token: SecretString::from("os.environ/ASSERTION"),
        azure_client_id: identity.map(str::to_owned),
        azure_tenant_id: identity.map(str::to_owned),
        scope: None,
    }
}

fn environment(credential: AzureCredentialType, filter: &str, identity: bool) -> ChainEnvironment {
    ChainEnvironment {
        scope: None,
        credential: Some(credential),
        client_id: identity.then(|| "client".to_owned()),
        tenant_id: identity.then(|| "tenant".to_owned()),
        client_secret: identity.then(|| SecretString::from("secret")),
        username: None,
        password: None,
        federated_token_file: None,
        certificate_path: None,
        certificate_password: None,
        client_certificate_path: None,
        client_certificate_password: None,
        client_send_certificate_chain: false,
        token_credentials: filter.to_owned(),
        authority_host: None,
    }
}

fn token_of(provider: &dyn CallableTokenProvider) -> Result<String, ProviderFailure> {
    match provider.token()? {
        TokenReply::Text(text) => Ok(text.expose_secret().to_owned()),
        TokenReply::NonText { type_name } => Ok(type_name),
    }
}

fn client_secret_record() -> ClientSecretRecord {
    ClientSecretRecord {
        tenant_id: "tenant".to_owned(),
        client_id: "client".to_owned(),
        client_secret: SecretString::from("secret"),
        scope: "scope".to_owned(),
    }
}

fn username_password_record() -> UsernamePasswordRecord {
    UsernamePasswordRecord {
        client_id: "client".to_owned(),
        username: "user".to_owned(),
        password: SecretString::from("pass"),
        tenant_id: None,
        scope: "scope".to_owned(),
    }
}

fn grant_sources<'data>(
    environment: &'data ChainEnvironment,
    grants: &'data FakeGrants,
    exchange: &'data AzureOidcExchange,
) -> GrantSources<'data, FakeGrants> {
    GrantSources {
        environment,
        grants,
        exchange,
        resolver: &FixedResolver,
    }
}

const WORKING: FakeGrants = FakeGrants {
    construct_error: None,
    link_error: None,
};

pub(super) fn grant_tokens() {
    let environment = ChainEnvironment::default();
    let exchange = exchange(200, "{}");
    let working = grant_sources(&environment, &WORKING, &exchange);
    let secret = working.client_secret(&client_secret_record()).unwrap();
    assert_eq!(token_of(&*secret).unwrap(), "client_secret-token");
    let password = working
        .username_password(&username_password_record())
        .unwrap();
    assert_eq!(token_of(&*password).unwrap(), "username_password-token");
}

pub(super) fn grant_construction_failures() {
    let environment = ChainEnvironment::default();
    let exchange = exchange(200, "{}");
    let rejected = FakeGrants {
        construct_error: Some("bad record"),
        link_error: None,
    };
    let sources = grant_sources(&environment, &rejected, &exchange);
    let failure = sources
        .client_secret(&client_secret_record())
        .err()
        .unwrap();
    assert_eq!(failure.class, "ValueError");
    assert_eq!(failure.status, None);
    assert_eq!(failure.message, "bad record");
    let password_failure = sources
        .username_password(&username_password_record())
        .err()
        .unwrap();
    assert_eq!(password_failure.class, "ValueError");
    assert_eq!(password_failure.status, None);
    assert_eq!(password_failure.message, "bad record");
}

pub(super) fn grant_link_failures() {
    let environment = ChainEnvironment::default();
    let exchange = exchange(200, "{}");
    for link_error in [
        LinkError::Authentication("denied".to_owned()),
        LinkError::Unavailable("denied".to_owned()),
    ] {
        let failing = FakeGrants {
            construct_error: None,
            link_error: Some(link_error),
        };
        let sources = grant_sources(&environment, &failing, &exchange);
        let secret = sources.client_secret(&client_secret_record()).unwrap();
        let failure = token_of(&*secret).unwrap_err();
        assert_eq!(failure.class, "ClientAuthenticationError");
        assert_eq!(failure.message, "denied");
        let password = sources
            .username_password(&username_password_record())
            .unwrap();
        let password_failure = token_of(&*password).unwrap_err();
        assert_eq!(password_failure.class, "ClientAuthenticationError");
        assert_eq!(password_failure.message, "denied");
    }
}

pub(super) fn oidc_exchange_results() {
    let environment = ChainEnvironment::default();
    let granted = exchange(200, r#"{"access_token":"minted","expires_in":3600}"#);
    let sources = grant_sources(&environment, &WORKING, &granted);
    let token = sources.oidc(&oidc_input(Some("identity"))).unwrap();
    assert_eq!(token.expose_secret(), "minted");
    let missing = sources.oidc(&oidc_input(None)).unwrap_err();
    assert_eq!(missing.class, "AzureOpenAIError");
    assert_eq!(missing.status, Some(422));
    assert_eq!(
        missing.message,
        "AZURE_CLIENT_ID and AZURE_TENANT_ID must be set"
    );
    let refused = exchange(503, "unavailable");
    let failing = grant_sources(&environment, &WORKING, &refused);
    let failure = failing.oidc(&oidc_input(Some("identity"))).unwrap_err();
    assert_eq!(failure.class, "AzureOpenAIError");
    assert_eq!(failure.status, Some(503));
}

pub(super) fn sdk_refresh_tokens() {
    let exchange = exchange(200, "{}");
    let ready = environment(AzureCredentialType::ClientSecretCredential, "", true);
    let working = grant_sources(&ready, &WORKING, &exchange);
    let provider = working.sdk_refresh("scope").unwrap();
    assert_eq!(token_of(&*provider).unwrap(), "client_secret-token");

    let rejected = FakeGrants {
        construct_error: Some("no identity"),
        link_error: None,
    };
    let construct_failure = grant_sources(&ready, &rejected, &exchange)
        .sdk_refresh("scope")
        .err()
        .unwrap();
    assert_eq!(construct_failure.class, "ValueError");
    assert_eq!(construct_failure.status, None);
    assert!(construct_failure.message.contains("no identity"));

    let failing = FakeGrants {
        construct_error: None,
        link_error: Some(LinkError::Authentication("denied".to_owned())),
    };
    let denying = grant_sources(&ready, &failing, &exchange);
    let denied_provider = denying.sdk_refresh("scope").unwrap();
    let link_failure = token_of(&*denied_provider).unwrap_err();
    assert_eq!(link_failure.class, "ClientAuthenticationError");
    assert!(link_failure.message.contains("denied"));
}

pub(super) fn sdk_refresh_error_classes() {
    let exchange = exchange(200, "{}");
    let cases = [
        (
            AzureCredentialType::ClientSecretCredential,
            "",
            false,
            "KeyError",
        ),
        (
            AzureCredentialType::UsernamePasswordCredential,
            "",
            true,
            "TypeError",
        ),
        (
            AzureCredentialType::DefaultAzureCredential,
            "bogus",
            true,
            "ValueError",
        ),
        (
            AzureCredentialType::DefaultAzureCredential,
            "dev",
            true,
            "RuntimeError",
        ),
    ];
    for (credential, filter, identity, class) in cases {
        let env = environment(credential, filter, identity);
        let failure = grant_sources(&env, &WORKING, &exchange)
            .sdk_refresh("scope")
            .err()
            .unwrap();
        assert_eq!(failure.class, class);
        assert_eq!(failure.status, None);
        assert!(!failure.message.is_empty());
    }
}

pub(super) fn default_credential_construction() {
    let exchange = exchange(200, "{}");
    let managed = environment(
        AzureCredentialType::DefaultAzureCredential,
        "managedidentitycredential",
        false,
    );
    let working = grant_sources(&managed, &WORKING, &exchange);
    let provider = working.default_credential("scope").unwrap();
    assert_eq!(token_of(&*provider).unwrap(), "managed_identity-token");
    let rejected = FakeGrants {
        construct_error: Some("no identity"),
        link_error: None,
    };
    assert!(
        grant_sources(&managed, &rejected, &exchange)
            .default_credential("scope")
            .is_none()
    );
}

pub(super) fn token_error_status() {
    let from_source = AzureTokenError::Source(SourceFailure {
        class: "AzureOpenAIError".to_owned(),
        status: Some(401),
        message: "denied".to_owned(),
    });
    assert_eq!(from_source.status(), Some(401));
    assert_eq!(from_source.class(), "AzureOpenAIError");
    let local = [
        AzureTokenError::TokenNotString {
            type_name: "int".to_owned(),
        },
        AzureTokenError::ProviderTypeError {
            message: "bad".to_owned(),
        },
        AzureTokenError::ProviderFailed {
            message: "bad".to_owned(),
        },
    ];
    for error in local {
        assert_eq!(error.status(), None);
    }
}
