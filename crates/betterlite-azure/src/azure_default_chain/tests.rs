#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::azure_default_chain::{AzureCredentialType, Settings};
use rstest::rstest;
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::environment::ChainEnvironment;
use super::error::DefaultChainError;
use super::grant::{
    CertificateRecord, ChainToken, ClientSecretRecord, Grant, GrantError, Grants, LinkError,
    ManagedIdentityRecord, UsernamePasswordRecord, WorkloadRecord,
};
use super::native::ResolvedValues;
use super::selection::selection;
use crate::azure_client_secret::ClientSecretInput;
use crate::azure_client_secret::provider::ProviderCache;
use crate::azure_client_secret::transport::{Clock, HttpReply, HttpRequest, Transport};
use betterlite_config::sections::azure_client_secret::Settings as ClientSecretSettings;

mod cases;
mod native;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Script {
    Token(String),
    Unavailable(String),
    Authentication(String),
}

#[derive(Debug, Clone, Default)]
pub(super) struct Recorded {
    pub(super) credential: String,
    pub(super) client_id: Option<String>,
    pub(super) tenant_id: Option<String>,
    pub(super) client_secret: Option<String>,
    pub(super) username: Option<String>,
    pub(super) password: Option<String>,
    pub(super) certificate_path: Option<String>,
    pub(super) send_certificate_chain: Option<bool>,
    pub(super) token_file: Option<String>,
    pub(super) exclude_workload_identity: Option<bool>,
    pub(super) imds_probe: Option<bool>,
    pub(super) within_credential_chain: Option<bool>,
    pub(super) within_default_credential: Option<bool>,
}

impl Recorded {
    fn blank(credential: &'static str) -> Self {
        Self {
            credential: credential.to_owned(),
            client_id: None,
            tenant_id: None,
            client_secret: None,
            username: None,
            password: None,
            certificate_path: None,
            send_certificate_chain: None,
            token_file: None,
            exclude_workload_identity: None,
            imds_probe: None,
            within_credential_chain: None,
            within_default_credential: None,
        }
    }

    fn secret(record: &ClientSecretRecord) -> Self {
        let mut recorded = Self::blank("ClientSecretCredential");
        recorded.client_id = Some(record.client_id.clone());
        recorded.tenant_id = Some(record.tenant_id.clone());
        recorded.client_secret = Some(exposed(&record.client_secret));
        recorded
    }

    fn certificate(record: &CertificateRecord) -> Self {
        Self {
            client_id: Some(record.client_id.clone()),
            tenant_id: Some(record.tenant_id.clone()),
            certificate_path: Some(record.certificate_path.clone()),
            password: record.password.as_ref().map(exposed),
            send_certificate_chain: Some(record.send_certificate_chain),
            ..Self::blank("CertificateCredential")
        }
    }

    fn with_user_password(mut self, record: &UsernamePasswordRecord) -> Self {
        self.client_id = Some(record.client_id.clone());
        self.tenant_id = record.tenant_id.clone();
        self.username = Some(record.username.clone());
        self.password = Some(exposed(&record.password));
        self
    }

    fn workload(record: &WorkloadRecord) -> Self {
        let mut recorded = Self::blank("WorkloadIdentityCredential");
        recorded.client_id = Some(record.client_id.clone());
        recorded.tenant_id = Some(record.tenant_id.clone());
        recorded.token_file = Some(record.token_file.clone());
        recorded
    }
}

pub(super) struct FixtureGrants {
    records: Mutex<Vec<Recorded>>,
    calls: Mutex<Vec<String>>,
    scripts: Vec<(String, Script)>,
    construct_errors: BTreeMap<String, String>,
}

impl FixtureGrants {
    pub(super) fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            scripts: Vec::new(),
            construct_errors: BTreeMap::new(),
        }
    }

    pub(super) fn script(mut self, credential: &str, script: Script) -> Self {
        self.scripts.push((credential.to_owned(), script));
        self
    }

    pub(super) fn construct_error(mut self, credential: &str, message: &str) -> Self {
        self.construct_errors
            .insert(credential.to_owned(), message.to_owned());
        self
    }

    pub(super) fn records(&self) -> Vec<Recorded> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn grant<'grants>(&'grants self, credential: &'static str) -> ScriptedGrant<'grants> {
        ScriptedGrant {
            grants: self,
            credential: credential.to_owned(),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, recorded: Recorded) -> Result<(), GrantError> {
        if let Some(message) = self.construct_errors.get(&recorded.credential) {
            return Err(GrantError {
                credential: "fixture",
                message: message.clone(),
            });
        }
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        Ok(())
    }

    fn scripted(&self, credential: &str) -> Script {
        self.scripts
            .iter()
            .find_map(|(name, script)| (name == credential).then(|| script.clone()))
            .unwrap_or_else(|| Script::Token(format!("{credential}-token")))
    }
}

pub(super) struct ScriptedGrant<'grants> {
    grants: &'grants FixtureGrants,
    credential: String,
}

impl Grant for ScriptedGrant<'_> {
    fn token(&self) -> Result<ChainToken, LinkError> {
        self.grants
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(self.credential.clone());
        match self.grants.scripted(&self.credential) {
            Script::Token(token) => Ok(ChainToken {
                token: SecretString::from(token),
                expires_on: None,
                token_type: None,
                refresh_on: None,
            }),
            Script::Unavailable(message) => Err(LinkError::Unavailable(message)),
            Script::Authentication(message) => Err(LinkError::Authentication(message)),
        }
    }
}

fn managed_identity_credential(record: &ManagedIdentityRecord) -> String {
    match record.client_id.as_deref() {
        Some(client_id) if !client_id.is_empty() => "user".to_owned(),
        Some(_) | None => "system".to_owned(),
    }
}

fn exposed(secret: &SecretString) -> String {
    secrecy::ExposeSecret::expose_secret(secret).to_owned()
}

impl Grants for FixtureGrants {
    type Grant<'grants>
        = ScriptedGrant<'grants>
    where
        Self: 'grants;

    fn client_secret<'grants>(
        &'grants self,
        record: &ClientSecretRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        self.record(Recorded::secret(record))?;
        Ok(self.grant("ClientSecretCredential"))
    }

    fn certificate<'grants>(
        &'grants self,
        record: &CertificateRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        self.record(Recorded::certificate(record))?;
        Ok(self.grant("CertificateCredential"))
    }

    fn username_password<'grants>(
        &'grants self,
        record: &UsernamePasswordRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        self.record(Recorded::blank("UsernamePasswordCredential").with_user_password(record))?;
        Ok(self.grant("UsernamePasswordCredential"))
    }

    fn workload<'grants>(
        &'grants self,
        record: &WorkloadRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        self.record(Recorded::workload(record))?;
        Ok(self.grant("WorkloadIdentityCredential"))
    }

    fn managed_identity<'grants>(
        &'grants self,
        record: &ManagedIdentityRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        let identity = managed_identity_credential(record);
        self.record(Recorded {
            credential: "ManagedIdentityCredential".to_owned(),
            client_id: record.client_id.clone(),
            exclude_workload_identity: Some(record.exclude_workload_identity),
            imds_probe: record.imds_probe,
            within_credential_chain: Some(record.placement.within_credential_chain()),
            within_default_credential: Some(record.placement.within_default_credential()),
            tenant_id: None,
            client_secret: None,
            username: None,
            password: None,
            certificate_path: None,
            send_certificate_chain: None,
            token_file: None,
        })?;
        Ok(ScriptedGrant {
            grants: self,
            credential: format!("ManagedIdentityCredential:{identity}"),
        })
    }
}

pub(super) fn environment(variables: &[(&str, &str)]) -> ChainEnvironment {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let settings: Settings = ConfigLoader {
        lookup: lookup.clone(),
    }
    .load("")
    .unwrap()
    .sections
    .azure_default_chain;
    ChainEnvironment::from_settings(&settings, &lookup)
}

pub(super) fn provider<'env, 'grants>(
    scope: Option<&'env str>,
    credential: Option<AzureCredentialType>,
    environment: &'env ChainEnvironment,
    grants: &'grants FixtureGrants,
) -> Result<super::provider::AzureTokenProvider<'grants, FixtureGrants>, DefaultChainError> {
    super::provider::get_azure_ad_token_provider(&super::provider::ProviderRequest {
        scope,
        credential,
        environment,
        grants,
    })
}

#[derive(Clone, Default)]
pub(super) struct Messages(Arc<Mutex<Vec<String>>>);

struct RawMessage(String);

impl Visit for RawMessage {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}").trim_matches('"').to_owned();
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for Messages {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        let mut message = RawMessage(String::new());
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(super) fn capture<T, F>(run: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let messages = Messages::default();
    let subscriber = tracing_subscriber::registry().with(messages.clone());
    let result = with_default(subscriber, run);
    let captured = messages
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (result, captured)
}

#[rstest]
#[case::explicit_credential(cases::explicit_credential)]
#[case::client_secret_inference(cases::client_secret_inference)]
#[case::client_id_before_certificate(cases::client_id_before_certificate)]
#[case::deployment_chain_order(cases::deployment_chain_order)]
#[case::default_chain_fallback(cases::default_chain_fallback)]
#[case::credential_constructor_arguments(cases::credential_constructor_arguments)]
#[case::default_environment_certificate(cases::default_environment_certificate)]
#[case::default_credential_filter(cases::default_credential_filter)]
#[case::default_provider_try_boundary(cases::default_provider_try_boundary)]
#[case::chain_exhaustion(cases::chain_exhaustion)]
#[case::generic_credential_constructor(cases::generic_credential_constructor)]
fn azure_default_chain_cases(#[case] case: fn()) {
    case();
}

#[test]
fn azure_default_chain_unsupported_credential_name() {
    let invalid_credential = ConfigLoader {
        lookup: BTreeMap::from([(
            "AZURE_CREDENTIAL".to_owned(),
            "AzureCliCredential".to_owned(),
        )]),
    }
    .load("")
    .unwrap_err();
    let ConfigError::InvalidScalar { path, .. } = &invalid_credential else {
        panic!("an unsupported credential name loaded");
    };
    assert!(path.contains("AZURE_CREDENTIAL=AzureCliCredential"));

    let developer_filter = ConfigLoader {
        lookup: BTreeMap::from([("AZURE_TOKEN_CREDENTIALS".to_owned(), "dev".to_owned())]),
    }
    .load("")
    .unwrap_err();
    let ConfigError::InvalidScalar {
        path: credential_path,
        ..
    } = &developer_filter
    else {
        panic!("a developer credential filter loaded");
    };
    assert!(credential_path.contains("AZURE_TOKEN_CREDENTIALS"));
    assert_eq!(selection("dev").unwrap_err().category(), "unsupported");
}

struct NoTransport;

impl Transport for NoTransport {
    fn send(&self, _request: &HttpRequest) -> io::Result<HttpReply> {
        Err(io::Error::other("no transport"))
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> i64 {
        1_700_000_000
    }
}

fn cache() -> ProviderCache {
    let transport: Arc<dyn Transport> = Arc::new(NoTransport);
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    ProviderCache::new(transport, clock, ClientSecretSettings::default())
}

fn credential_input(tenant_id: &str, client_id: &str, secret: &str) -> ClientSecretInput {
    ClientSecretInput {
        tenant_id: tenant_id.to_owned(),
        client_id: client_id.to_owned(),
        client_secret: SecretString::from(secret),
        scope: None,
    }
}

#[test]
fn azure_default_chain_credential_identity_isolation() {
    cache_isolates_identities();
    configured_identities_do_not_mix();
}

fn cache_isolates_identities() {
    let mut providers = cache();
    let first = providers
        .provider(
            &credential_input("tenant-a", "client-a", "secret-a"),
            &ResolvedValues,
            None,
        )
        .unwrap();
    let same = providers
        .provider(
            &credential_input("tenant-a", "client-a", "secret-a"),
            &ResolvedValues,
            None,
        )
        .unwrap();
    let other = providers
        .provider(
            &credential_input("tenant-b", "client-b", "secret-b"),
            &ResolvedValues,
            None,
        )
        .unwrap();
    let rotated = providers
        .provider(
            &credential_input("tenant-a", "client-a", "secret-b"),
            &ResolvedValues,
            None,
        )
        .unwrap();
    assert!(Arc::ptr_eq(&first, &same));
    assert!(!Arc::ptr_eq(&first, &other));
    assert!(!Arc::ptr_eq(&first, &rotated));
}

fn configured_identities_do_not_mix() {
    let grants = FixtureGrants::new();
    let first = environment(&[
        ("AZURE_CLIENT_ID", "client-a"),
        ("AZURE_CLIENT_SECRET", "secret-a"),
        ("AZURE_TENANT_ID", "tenant-a"),
    ]);
    let second = environment(&[
        ("AZURE_CLIENT_ID", "client-b"),
        ("AZURE_CLIENT_SECRET", "secret-b"),
        ("AZURE_TENANT_ID", "tenant-b"),
    ]);
    provider(None, None, &first, &grants).unwrap();
    provider(None, None, &second, &grants).unwrap();
    let records = grants.records();
    assert_eq!(records[0].client_id.as_deref(), Some("client-a"));
    assert_eq!(records[0].client_secret.as_deref(), Some("secret-a"));
    assert_eq!(records[1].client_id.as_deref(), Some("client-b"));
    assert_eq!(records[1].client_secret.as_deref(), Some("secret-b"));
}

#[test]
fn azure_default_chain_secret_safe_failure() {
    records_hide_secrets();
    failures_hide_secrets();
}

#[test]
fn provider_debug_names_the_credential_and_scope_only() {
    let grants = FixtureGrants::new();
    let env = environment(&[]);
    let built = provider(
        Some("https://example.test/.default"),
        Some(AzureCredentialType::DefaultAzureCredential),
        &env,
        &grants,
    )
    .unwrap();
    let text = format!("{built:?}");
    assert!(text.starts_with("AzureTokenProvider {"));
    assert!(text.contains("DefaultAzureCredential"));
    assert!(text.contains("https://example.test/.default"));
    assert!(!text.contains("inner"));
}

fn records_hide_secrets() {
    let secret = SecretString::from("fixture-super-secret");
    let client_secret = ClientSecretRecord {
        tenant_id: "tenant".to_owned(),
        client_id: "client".to_owned(),
        client_secret: secret.clone(),
        scope: String::new(),
    };
    let certificate = CertificateRecord {
        tenant_id: "tenant".to_owned(),
        client_id: "client".to_owned(),
        certificate_path: "/certs/azure.pem".to_owned(),
        password: Some(secret.clone()),
        send_certificate_chain: false,
        scope: String::new(),
    };
    let username_password = UsernamePasswordRecord {
        client_id: "client".to_owned(),
        username: "user@example.test".to_owned(),
        password: secret,
        tenant_id: None,
        scope: String::new(),
    };
    let workload = WorkloadRecord {
        client_id: "client".to_owned(),
        tenant_id: "tenant".to_owned(),
        token_file: "/var/run/token".to_owned(),
        scope: String::new(),
    };
    let records: [&dyn fmt::Debug; 4] =
        [&client_secret, &certificate, &username_password, &workload];
    for record in records {
        assert!(!format!("{record:?}").contains("fixture-super-secret"));
    }
}

fn failures_hide_secrets() {
    let token = ChainToken {
        token: SecretString::from("fixture-super-secret"),
        expires_on: None,
        token_type: None,
        refresh_on: None,
    };
    assert!(!format!("{token:?}").contains("fixture-super-secret"));
    let error = DefaultChainError::Credential {
        unavailable: false,
        message: "denied".to_owned(),
    };
    assert!(!format!("{error:?}").contains("fixture-super-secret"));
    let record = ManagedIdentityRecord {
        client_id: Some("client".to_owned()),
        exclude_workload_identity: false,
        imds_probe: None,
        placement: super::grant::ChainPlacement::Standalone,
        scope: String::new(),
    };
    assert!(!format!("{record:?}").contains("fixture-super-secret"));
}
