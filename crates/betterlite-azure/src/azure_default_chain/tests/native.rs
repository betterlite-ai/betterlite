use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_config::sections::azure_client_secret::Settings as ClientSecretSettings;
use betterlite_config::sections::azure_oidc_exchange::Settings as OidcSettings;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::FixedClock;
use crate::azure_arc_identity::{ArcHost, ArcPlatform, ArcRequest, ArcTransport, MetadataResponse};
use crate::azure_certificate::assertion::IdSource;
use crate::azure_certificate::credential::FileReader;
use crate::azure_client_secret::ClientSecretError;
use crate::azure_client_secret::provider::ProviderCache;
use crate::azure_client_secret::transport::{Clock, HttpMethod, HttpReply, HttpRequest, Transport};
use crate::azure_default_chain::grant::{
    CertificateRecord, ChainPlacement, ChainToken, ClientSecretRecord, Grant as _, Grants as _,
    LinkError, ManagedIdentityRecord, UsernamePasswordRecord, WorkloadRecord,
};
use crate::azure_default_chain::native::NativeGrants;
use crate::azure_hosted_identity::request::{
    HostedIo, HostedRequest, HostedResponse, HostedTransport,
};
use crate::azure_managed_identity::environment::ManagedIdentityEnvironment;
use crate::azure_managed_identity::imds::{ImdsRequest, ImdsTransport};
use crate::azure_managed_identity::runtime::{ImdsIo, Scheduler, TokenFiles};
use crate::azure_username_password::transport::{
    HttpRequest as PasswordRequest, Transport as PasswordTransport,
};
use crate::oidc_exchange::AzureOidcExchange;
use crate::oidc_exchange::transport::{
    Clock as OidcClock, HttpReply as OidcReply, Transport as OidcTransport,
};

const NOW: i64 = 1_700_000_000;
const SCOPE: &str = "https://cognitiveservices.azure.com/.default";
const CLAIMS: &str = r#""id_token_claims":{"aud":"client","iss":"https://login.microsoftonline.com/tenant/v2.0","sub":"subject","tid":"tenant","preferred_username":"user@example.com"},"client_info":"eyJ1aWQiOiJ1aWQiLCJ1dGlkIjoidXRpZCJ9""#;
const CERTIFICATE_KEY: &[u8] =
    include_bytes!("../../azure_certificate/tests/fixtures/rsa_encrypted.pem");
const CERTIFICATE_PASSWORD: &str = "fixture-password";

#[derive(Clone)]
struct Seen {
    method: HttpMethod,
    url: String,
    form: Vec<(String, String)>,
}

struct Wire {
    expires_in: i64,
    extra: &'static str,
    seen: Mutex<Vec<Seen>>,
}

impl Wire {
    fn requests(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn posts(&self) -> Vec<Seen> {
        let all = self.requests();
        all.into_iter()
            .filter(|seen| seen.method == HttpMethod::Post)
            .collect()
    }

    fn answer(
        &self,
        method: HttpMethod,
        url: &str,
        form: &[(String, String)],
    ) -> io::Result<HttpReply> {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.push(Seen {
            method,
            url: url.to_owned(),
            form: form.to_vec(),
        });
        let posts = seen
            .iter()
            .filter(|request| request.method == HttpMethod::Post)
            .count();
        drop(seen);
        let body = if url.ends_with("/.well-known/openid-configuration") {
            let mut parts = url.trim_start_matches("https://").split('/');
            let host = parts.next().unwrap_or_default();
            let tenant = parts.next().unwrap_or_default();
            format!(
                r#"{{"issuer":"https://{host}/{tenant}/v2.0","authorization_endpoint":"https://{host}/{tenant}/oauth2/v2.0/authorize","token_endpoint":"https://{host}/{tenant}/oauth2/v2.0/token"}}"#
            )
        } else if url.contains("/oauth2/v2.0/token") {
            format!(
                r#"{{"access_token":"tok-{posts}","expires_in":{},{}{CLAIMS}}}"#,
                self.expires_in, self.extra
            )
        } else if url.contains("/common/userrealm/") {
            "{}".to_owned()
        } else {
            return Err(io::Error::other(format!("no reply for {url}")));
        };
        Ok(HttpReply {
            status: 200,
            body: SecretString::from(body),
        })
    }
}

impl Transport for Wire {
    fn send(&self, request: &HttpRequest) -> io::Result<HttpReply> {
        self.answer(request.method, &request.url, &request.form)
    }
}

impl PasswordTransport for Wire {
    fn send(&self, request: &PasswordRequest) -> io::Result<HttpReply> {
        self.answer(request.method, &request.url, &request.form)
    }
}

struct Files(BTreeMap<String, Vec<u8>>);

impl FileReader for Files {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.0
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }
}

struct Tokens(BTreeMap<String, String>);

impl TokenFiles for Tokens {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.0
            .get(path)
            .map(|text| text.as_bytes().to_vec())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such token file"))
    }
}

struct FixedId;

impl IdSource for FixedId {
    fn next_id(&self) -> String {
        "jti-1".to_owned()
    }
}

struct NoSleep;

impl Scheduler for NoSleep {
    fn sleep(&self, _delay: Duration) {}
}

struct QuietHost;

impl ArcHost for QuietHost {
    fn platform(&self) -> ArcPlatform {
        ArcPlatform::Other("test".to_owned())
    }

    fn environment(&self, _name: &str) -> Option<String> {
        None
    }

    fn program_data(&self) -> String {
        String::new()
    }

    fn program_files(&self) -> String {
        String::new()
    }

    fn file_size(&self, _path: &str) -> io::Result<u64> {
        Err(io::Error::other("no file"))
    }

    fn read_text(&self, _path: &str) -> io::Result<String> {
        Err(io::Error::other("no file"))
    }

    fn now_unix_seconds(&self) -> i64 {
        NOW
    }
}

struct NoArc;

impl ArcTransport for NoArc {
    fn get(&self, _request: &ArcRequest) -> io::Result<MetadataResponse> {
        Err(io::Error::other("arc is not reachable"))
    }
}

struct DownHosted(AtomicUsize);

impl HostedTransport for DownHosted {
    fn send(&self, _request: &HostedRequest) -> io::Result<HostedResponse> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::other("hosted endpoint is down"))
    }
}

// each entry is true for the availability probe and false for the token request
struct Imds {
    down: bool,
    requests: Mutex<Vec<bool>>,
    client_ids: Mutex<Vec<Option<String>>>,
}

impl ImdsTransport for Imds {
    fn get(&self, request: &ImdsRequest) -> io::Result<MetadataResponse> {
        let probe = !request.headers.iter().any(|(name, _)| name == "Metadata");
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(probe);
        let client_id = request
            .query
            .iter()
            .find(|(name, _)| name == "client_id")
            .map(|(_, value)| value.clone());
        self.client_ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(client_id);
        if self.down {
            return Err(io::Error::other("imds is down"));
        }
        let body = if probe {
            ""
        } else {
            r#"{"access_token":"imds-token","expires_in":3600}"#
        };
        Ok(MetadataResponse {
            status: 200,
            headers: Vec::new(),
            body: SecretString::from(body),
        })
    }
}

struct OidcWire {
    requests: Mutex<Vec<(String, BTreeMap<String, String>)>>,
}

impl OidcTransport for OidcWire {
    fn post_form(&self, url: &str, form: &[(String, SecretString)]) -> io::Result<OidcReply> {
        let fields = form
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.to_owned(), fields));
        Ok(OidcReply {
            status: 200,
            body: SecretString::from(
                r#"{"access_token":"aad-token","expires_in":3600,"token_type":"Bearer"}"#,
            ),
        })
    }
}

struct OidcNow;

impl OidcClock for OidcNow {
    fn now_unix_seconds(&self) -> f64 {
        1_700_000_000.0
    }
}

struct Parts {
    wire: Arc<Wire>,
    clock: Arc<FixedClock>,
    files: Files,
    tokens: Tokens,
    environment: ManagedIdentityEnvironment,
    host: QuietHost,
    hosted: DownHosted,
    arc: NoArc,
    imds: Imds,
    scheduler: NoSleep,
    ids: FixedId,
    redactor: Redactor,
    oidc: Arc<OidcWire>,
    exchange: AzureOidcExchange,
}

impl Parts {
    fn new(expires_in: i64, extra: &'static str) -> Self {
        let oidc = Arc::new(OidcWire {
            requests: Mutex::new(Vec::new()),
        });
        let recorded_oidc: Arc<OidcWire> = Arc::clone(&oidc);
        let oidc_transport: Arc<dyn OidcTransport> = recorded_oidc;
        let exchange = AzureOidcExchange::new(
            OidcSettings {
                azure_authority_host: "https://login.contoso.test".to_owned(),
            },
            BTreeMap::new(),
            oidc_transport,
            Arc::new(OidcNow),
        )
        .unwrap();
        Self {
            wire: Arc::new(Wire {
                expires_in,
                extra,
                seen: Mutex::new(Vec::new()),
            }),
            clock: Arc::new(FixedClock),
            files: Files(BTreeMap::new()),
            tokens: Tokens(BTreeMap::new()),
            environment: ManagedIdentityEnvironment::default(),
            host: QuietHost,
            hosted: DownHosted(AtomicUsize::new(0)),
            arc: NoArc,
            imds: Imds {
                down: false,
                requests: Mutex::new(Vec::new()),
                client_ids: Mutex::new(Vec::new()),
            },
            scheduler: NoSleep,
            ids: FixedId,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
            oidc,
            exchange,
        }
    }

    fn grants(&self) -> NativeGrants<'_, QuietHost> {
        let shared_wire: Arc<Wire> = Arc::clone(&self.wire);
        let transport: Arc<dyn Transport> = shared_wire;
        let shared_clock: Arc<FixedClock> = Arc::clone(&self.clock);
        let clock: Arc<dyn Clock> = shared_clock;
        NativeGrants {
            transport: Arc::clone(&transport),
            clock: Arc::clone(&clock),
            client_secrets: Mutex::new(ProviderCache::new(
                transport,
                clock,
                ClientSecretSettings::default(),
            )),
            files: &self.files,
            ids: Arc::new(FixedId),
            token_files: &self.tokens,
            exchange: &self.exchange,
            managed_environment: &self.environment,
            host: &self.host,
            hosted: HostedIo {
                transport: &self.hosted,
                clock: &*self.clock,
                redactor: &self.redactor,
            },
            arc: &self.arc,
            imds: ImdsIo {
                transport: &self.imds,
                scheduler: &self.scheduler,
                ids: &self.ids,
                clock: &*self.clock,
            },
            password_transport: &*self.wire,
            authority_host: None,
        }
    }

    fn imds_requests(&self) -> Vec<bool> {
        self.imds
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn client_secret_record(secret: &str) -> ClientSecretRecord {
    ClientSecretRecord {
        tenant_id: "contoso".to_owned(),
        client_id: "client-1".to_owned(),
        client_secret: SecretString::from(secret),
        scope: "api://scope/.default".to_owned(),
    }
}

fn certificate_record(send_certificate_chain: bool) -> CertificateRecord {
    CertificateRecord {
        tenant_id: "tenant-1".to_owned(),
        client_id: "client-1".to_owned(),
        certificate_path: "/certs/leaf.pem".to_owned(),
        password: Some(SecretString::from(CERTIFICATE_PASSWORD)),
        send_certificate_chain,
        scope: "api://scope/.default".to_owned(),
    }
}

fn password_record(tenant_id: Option<&str>) -> UsernamePasswordRecord {
    UsernamePasswordRecord {
        client_id: "client".to_owned(),
        username: "user@example.com".to_owned(),
        password: SecretString::from("fixture-pw"),
        tenant_id: tenant_id.map(str::to_owned),
        scope: SCOPE.to_owned(),
    }
}

fn workload_record() -> WorkloadRecord {
    WorkloadRecord {
        client_id: "client-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        token_file: "/var/token".to_owned(),
        scope: "api://scope/.default".to_owned(),
    }
}

fn identity_record(placement: ChainPlacement) -> ManagedIdentityRecord {
    ManagedIdentityRecord {
        client_id: None,
        exclude_workload_identity: false,
        imds_probe: None,
        placement,
        scope: SCOPE.to_owned(),
    }
}

fn form_value(seen: &Seen, name: &str) -> Option<String> {
    seen.form
        .iter()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value.clone())
}

fn token_text(token: &ChainToken) -> &str {
    token.token.expose_secret()
}

#[test]
fn native_client_secret_grant_exchanges_the_literal_secret() {
    let parts = Parts::new(3600, "");
    let grants = parts.grants();
    let grant = grants
        .client_secret(&client_secret_record("s3cret"))
        .unwrap();
    let token = grant.token().unwrap();
    assert_eq!(token_text(&token), "tok-1");
    assert_eq!(token.expires_on, Some(NOW + 3600));
    assert_eq!(token.token_type.as_deref(), Some("Bearer"));
    assert_eq!(token.refresh_on, None);
    let posts = parts.wire.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(
        posts[0].url,
        "https://login.microsoftonline.com/contoso/oauth2/v2.0/token"
    );
    assert_eq!(
        form_value(&posts[0], "client_id").as_deref(),
        Some("client-1")
    );
    assert_eq!(
        form_value(&posts[0], "client_secret").as_deref(),
        Some("s3cret")
    );
}

#[test]
fn native_client_secret_reference_is_never_resolved() {
    let parts = Parts::new(3600, "");
    let grants = parts.grants();
    let result = grants.client_secret(&client_secret_record("os.environ/UNRESOLVED"));
    let error = result.err().expect("an unresolved reference must fail");
    assert_eq!(error.credential, "ClientSecretCredential");
    assert_eq!(
        error.message,
        ClientSecretError::MissingCredentials.to_string()
    );
    assert!(parts.wire.requests().is_empty());
}

#[test]
fn native_client_secret_poisoned_cache_names_the_credential() {
    let parts = Parts::new(3600, "");
    let grants = parts.grants();
    thread::scope(|scope| {
        let poisoner = scope.spawn(|| {
            let _guard = grants.client_secrets.lock().unwrap();
            panic!("poison the provider cache");
        });
        assert!(poisoner.join().is_err());
    });
    let result = grants.client_secret(&client_secret_record("s3cret"));
    let error = result.err().expect("a poisoned cache must fail");
    assert_eq!(error.credential, "ClientSecretCredential");
    assert_eq!(error.message, "the provider cache is poisoned");
}

#[test]
fn native_certificate_grant_reads_the_path_and_signs_for_the_record() {
    let mut parts = Parts::new(3600, "");
    parts
        .files
        .0
        .insert("/certs/leaf.pem".to_owned(), CERTIFICATE_KEY.to_vec());
    let grants = parts.grants();
    let grant = grants.certificate(&certificate_record(true)).unwrap();
    let token = grant.token().unwrap();
    assert_eq!(token_text(&token), "tok-1");
    assert_eq!(token.expires_on, Some(NOW + 3600));
    assert_eq!(token.token_type.as_deref(), Some("Bearer"));
    let posts = parts.wire.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(
        posts[0].url,
        "https://login.microsoftonline.com/tenant-1/oauth2/v2.0/token"
    );
    assert_eq!(
        form_value(&posts[0], "client_id").as_deref(),
        Some("client-1")
    );
    assert_eq!(
        form_value(&posts[0], "scope").as_deref(),
        Some("api://scope/.default")
    );
}

#[derive(Deserialize)]
struct AssertionHeader {
    x5c: Option<Vec<String>>,
}

fn assertion_header(send_certificate_chain: bool) -> AssertionHeader {
    let mut parts = Parts::new(3600, "");
    parts
        .files
        .0
        .insert("/certs/leaf.pem".to_owned(), CERTIFICATE_KEY.to_vec());
    let grants = parts.grants();
    let grant = grants
        .certificate(&certificate_record(send_certificate_chain))
        .unwrap();
    grant.token().unwrap();
    let posts = parts.wire.posts();
    let assertion = form_value(&posts[0], "client_assertion").unwrap();
    let segment = assertion.split('.').next().unwrap();
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(segment).unwrap()).unwrap()
}

#[test]
fn native_certificate_grant_passes_the_chain_flag_and_password() {
    let sent = assertion_header(true);
    assert!(sent.x5c.is_some_and(|chain| !chain.is_empty()));
    assert!(assertion_header(false).x5c.is_none());
}

#[test]
fn native_certificate_grant_failure_names_the_path() {
    let parts = Parts::new(3600, "");
    let grants = parts.grants();
    let result = grants.certificate(&certificate_record(false));
    let error = result.err().expect("a missing file must fail");
    assert_eq!(error.credential, "CertificateCredential");
    assert!(error.message.contains("/certs/leaf.pem"));
}

#[rstest]
#[case::absent(None, "organizations")]
#[case::empty(Some(""), "organizations")]
#[case::named(Some("contoso"), "contoso")]
fn native_password_grant_defaults_the_tenant_to_organizations(
    #[case] tenant: Option<&str>,
    #[case] expected: &str,
) {
    let parts = Parts::new(3600, "");
    let grants = parts.grants();
    let grant = grants.username_password(&password_record(tenant)).unwrap();
    let token = grant.token().unwrap();
    assert_eq!(token_text(&token), "tok-1");
    let requests = parts.wire.requests();
    assert_eq!(
        requests[0].url,
        format!(
            "https://login.microsoftonline.com/{expected}/v2.0/.well-known/openid-configuration"
        )
    );
    let posts = parts.wire.posts();
    assert_eq!(
        posts[0].url,
        format!("https://login.microsoftonline.com/{expected}/oauth2/v2.0/token")
    );
    assert_eq!(
        form_value(&posts[0], "username").as_deref(),
        Some("user@example.com")
    );
    assert_eq!(
        form_value(&posts[0], "password").as_deref(),
        Some("fixture-pw")
    );
    assert_eq!(
        form_value(&posts[0], "client_id").as_deref(),
        Some("client")
    );
}

#[test]
fn native_password_grant_reports_the_lifetime_fields() {
    let parts = Parts::new(3600, r#""refresh_in":1200,"#);
    let grants = parts.grants();
    let grant = grants.username_password(&password_record(None)).unwrap();
    let token = grant.token().unwrap();
    assert_eq!(token_text(&token), "tok-1");
    assert_eq!(token.expires_on, Some(NOW + 3600));
    assert_eq!(token.token_type.as_deref(), Some("Bearer"));
    assert_eq!(token.refresh_on, Some(NOW + 1200));
}

#[test]
fn native_password_grant_keeps_a_token_with_five_minutes_left() {
    let parts = Parts::new(300, "");
    let grants = parts.grants();
    let grant = grants.username_password(&password_record(None)).unwrap();
    let first = grant.token().unwrap();
    let second = grant.token().unwrap();
    assert_eq!(token_text(&first), "tok-1");
    assert_eq!(token_text(&second), "tok-1");
    assert_eq!(parts.wire.posts().len(), 1);
}

#[test]
fn native_password_grant_renews_a_token_inside_five_minutes_and_keeps_the_authority() {
    let parts = Parts::new(299, "");
    let grants = parts.grants();
    let grant = grants.username_password(&password_record(None)).unwrap();
    let first = grant.token().unwrap();
    let second = grant.token().unwrap();
    assert_eq!(token_text(&first), "tok-1");
    assert_eq!(token_text(&second), "tok-2");
    assert_eq!(parts.wire.posts().len(), 2);
    let discoveries = parts
        .wire
        .requests()
        .iter()
        .filter(|seen| seen.url.ends_with("/.well-known/openid-configuration"))
        .count();
    assert_eq!(discoveries, 1);
}

#[test]
fn native_workload_grant_exchanges_the_token_file() {
    let mut parts = Parts::new(3600, "");
    parts
        .tokens
        .0
        .insert("/var/token".to_owned(), "file-jwt".to_owned());
    let grants = parts.grants();
    let grant = grants.workload(&workload_record()).unwrap();
    let token = grant.token().unwrap();
    assert_eq!(token_text(&token), "aad-token");
    assert_eq!(token.expires_on, None);
    assert_eq!(token.token_type, None);
    assert_eq!(token.refresh_on, None);
    let requests = parts.oidc.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].0,
        "https://login.contoso.test/tenant-1/oauth2/v2.0/token"
    );
    assert_eq!(
        requests[0].1.get("client_id").map(String::as_str),
        Some("client-1")
    );
    assert_eq!(
        requests[0].1.get("client_assertion").map(String::as_str),
        Some("file-jwt")
    );
    assert_eq!(
        requests[0].1.get("scope").map(String::as_str),
        Some("api://scope/.default")
    );
}

#[test]
fn native_workload_grant_failure_stops_the_chain() {
    let parts = Parts::new(3600, "");
    let grants = parts.grants();
    let grant = grants.workload(&workload_record()).unwrap();
    let error = grant.token().unwrap_err();
    assert!(matches!(error, LinkError::Authentication(_)));
}

#[test]
fn native_managed_identity_grant_returns_the_imds_token() {
    let parts = Parts::new(3600, "");
    let grants = parts.grants();
    let grant = grants
        .managed_identity(&identity_record(ChainPlacement::Standalone))
        .unwrap();
    let token = grant.token().unwrap();
    assert_eq!(token_text(&token), "imds-token");
    assert_eq!(token.expires_on, Some(NOW + 3600));
}

#[test]
fn native_managed_identity_unreachable_imds_lets_the_chain_continue() {
    let mut parts = Parts::new(3600, "");
    parts.imds.down = true;
    let grants = parts.grants();
    let grant = grants
        .managed_identity(&identity_record(ChainPlacement::Standalone))
        .unwrap();
    let error = grant.token().unwrap_err();
    assert!(matches!(error, LinkError::Unavailable(_)));
}

#[rstest]
#[case::standalone(ChainPlacement::Standalone, vec![false])]
#[case::chained(ChainPlacement::Chained, vec![true, false])]
#[case::default_credential(ChainPlacement::DefaultCredential, vec![true, false])]
fn native_managed_identity_probes_imds_only_inside_a_chain(
    #[case] placement: ChainPlacement,
    #[case] expected: Vec<bool>,
) {
    let parts = Parts::new(3600, "");
    let grants = parts.grants();
    let grant = grants
        .managed_identity(&identity_record(placement))
        .unwrap();
    grant.token().unwrap();
    assert_eq!(parts.imds_requests(), expected);
}

#[test]
fn native_managed_identity_probe_setting_overrides_the_placement() {
    let parts = Parts::new(3600, "");
    let grants = parts.grants();
    let mut record = identity_record(ChainPlacement::Chained);
    record.imds_probe = Some(false);
    grants.managed_identity(&record).unwrap().token().unwrap();
    assert_eq!(parts.imds_requests(), vec![false]);
    let other = Parts::new(3600, "");
    let other_grants = other.grants();
    let mut forced = identity_record(ChainPlacement::Standalone);
    forced.imds_probe = Some(true);
    other_grants
        .managed_identity(&forced)
        .unwrap()
        .token()
        .unwrap();
    assert_eq!(other.imds_requests(), vec![true, false]);
}

#[rstest]
#[case::standalone(ChainPlacement::Standalone, 0)]
#[case::chained(ChainPlacement::Chained, 0)]
#[case::default_credential(ChainPlacement::DefaultCredential, 1)]
fn native_managed_identity_cloud_shell_accepts_a_client_id_only_under_default_credential(
    #[case] placement: ChainPlacement,
    #[case] sent: usize,
) {
    let mut parts = Parts::new(3600, "");
    parts.environment.hosted.msi_endpoint = Some("http://shell.test/token".to_owned());
    let grants = parts.grants();
    let mut record = identity_record(placement);
    record.client_id = Some("user-client".to_owned());
    let error = grants
        .managed_identity(&record)
        .unwrap()
        .token()
        .unwrap_err();
    assert!(matches!(error, LinkError::Authentication(_)));
    assert_eq!(parts.hosted.0.load(Ordering::SeqCst), sent);
}

#[rstest]
#[case::workload_first(false, "aad-token")]
#[case::workload_excluded(true, "imds-token")]
fn native_managed_identity_workload_selection_follows_the_record(
    #[case] exclude: bool,
    #[case] expected: &str,
) {
    let mut parts = Parts::new(3600, "");
    parts.environment.azure_authority_host = Some("https://login.contoso.test".to_owned());
    parts.environment.azure_tenant_id = Some("tenant-1".to_owned());
    parts.environment.azure_federated_token_file = Some("/var/token".to_owned());
    parts.environment.azure_client_id = Some("client-1".to_owned());
    parts
        .tokens
        .0
        .insert("/var/token".to_owned(), "file-jwt".to_owned());
    let grants = parts.grants();
    let mut record = identity_record(ChainPlacement::Standalone);
    record.exclude_workload_identity = exclude;
    let token = grants.managed_identity(&record).unwrap().token().unwrap();
    assert_eq!(token_text(&token), expected);
}

#[test]
fn native_managed_identity_client_id_selects_the_user_assigned_identity() {
    let parts = Parts::new(3600, "");
    let grants = parts.grants();
    let mut record = identity_record(ChainPlacement::Standalone);
    record.client_id = Some("user-client".to_owned());
    grants.managed_identity(&record).unwrap().token().unwrap();
    assert_eq!(parts.imds_requests(), vec![false]);
    let client_ids = parts.imds.client_ids.lock().unwrap();
    assert_eq!(*client_ids, vec![Some("user-client".to_owned())]);
}

#[test]
fn native_chain_placement_flags_follow_the_variant() {
    assert!(!ChainPlacement::Standalone.within_credential_chain());
    assert!(ChainPlacement::Chained.within_credential_chain());
    assert!(ChainPlacement::DefaultCredential.within_credential_chain());
    assert!(!ChainPlacement::Standalone.within_default_credential());
    assert!(!ChainPlacement::Chained.within_default_credential());
    assert!(ChainPlacement::DefaultCredential.within_default_credential());
}
