#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_db::mcp_server_encoding::credentials::CredentialBlob;
use betterlite_db::mcp_stored_contracts::{JsonValue, MCPAuth};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::server_record::OAuth2Flow;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredValue};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tracing_subscriber::fmt::MakeWriter;

use super::flow::backfill_null_oauth2_flows;
use super::issuer::heal_stamped_issuers;
use super::log::{BackfillEvent, BackfillLog, TracingBackfillLog};
use super::{
    BackfillRule, DISCOVERY_ACTOR, FlowBackfillTable, FlowCandidateRow, FlowSignals,
    IssuerBackfillTable, IssuerCandidateRow, UserCredentialRow, classify_null_flow_row,
    decrypted_credentials, flow_name, is_stamped_issuer_row, origin,
};

#[derive(Debug, thiserror::Error)]
#[error("the fake table failed")]
struct FakeError;

fn test_cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("salt-one".to_owned())),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn credential_fields(client_id: Option<&str>, client_secret: Option<&str>) -> CredentialBlob {
    let mut fields = CredentialBlob::new();
    if let Some(id) = client_id {
        fields.insert("client_id".to_owned(), JsonValue::String(id.to_owned()));
    }
    if let Some(secret) = client_secret {
        fields.insert(
            "client_secret".to_owned(),
            JsonValue::String(secret.to_owned()),
        );
    }
    fields
}

fn credentials_value(client_id: Option<&str>, client_secret: Option<&str>) -> Option<JsonValue> {
    Some(JsonValue::Object(credential_fields(
        client_id,
        client_secret,
    )))
}

fn oauth_payload_b64() -> String {
    STANDARD.encode(r#"{"type":"oauth2","access_token":"tok"}"#)
}

fn flow_row(server_id: &str) -> FlowCandidateRow {
    FlowCandidateRow {
        server_id: server_id.to_owned(),
        authorization_url: None,
        registration_url: None,
        token_url: None,
        credentials: None,
    }
}

fn signals<'row>(
    authorization_url: Option<&'row str>,
    registration_url: Option<&'row str>,
    token_url: Option<&'row str>,
    credentials: Option<&'row CredentialBlob>,
) -> FlowSignals<'row> {
    FlowSignals {
        has_per_user_tokens: false,
        authorization_url,
        registration_url,
        token_url,
        credentials,
    }
}

struct FakeFlowTable {
    rows: Vec<FlowCandidateRow>,
    tokens: Vec<UserCredentialRow>,
    stamps: Mutex<Vec<(Vec<String>, OAuth2Flow)>>,
}

impl FakeFlowTable {
    fn new(rows: Vec<FlowCandidateRow>, tokens: Vec<UserCredentialRow>) -> Self {
        Self {
            rows,
            tokens,
            stamps: Mutex::new(Vec::new()),
        }
    }

    fn stamps(&self) -> Vec<(Vec<String>, OAuth2Flow)> {
        self.stamps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl FlowBackfillTable for FakeFlowTable {
    type Error = FakeError;

    async fn null_flow_rows(&self) -> Result<Vec<FlowCandidateRow>, FakeError> {
        Ok(self.rows.clone())
    }

    async fn user_credentials(
        &self,
        server_ids: &[String],
    ) -> Result<Vec<UserCredentialRow>, FakeError> {
        Ok(self
            .tokens
            .iter()
            .filter(|row| server_ids.contains(&row.server_id))
            .cloned()
            .collect())
    }

    async fn stamp_flow(&self, server_ids: &[String], flow: OAuth2Flow) -> Result<(), FakeError> {
        self.stamps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((server_ids.to_vec(), flow));
        Ok(())
    }
}

struct FakeIssuerTable {
    rows: Mutex<Vec<IssuerCandidateRow>>,
    fail_for: Option<String>,
    cleared: Mutex<Vec<String>>,
}

impl FakeIssuerTable {
    fn new(rows: Vec<IssuerCandidateRow>, fail_for: Option<&str>) -> Self {
        Self {
            rows: Mutex::new(rows),
            fail_for: fail_for.map(str::to_owned),
            cleared: Mutex::new(Vec::new()),
        }
    }

    fn cleared(&self) -> Vec<String> {
        self.cleared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl IssuerBackfillTable for FakeIssuerTable {
    type Error = FakeError;

    async fn discovery_stamped_rows(&self) -> Result<Vec<IssuerCandidateRow>, FakeError> {
        Ok(self
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn clear_issuer(&self, server_id: &str) -> Result<(), FakeError> {
        if self.fail_for.as_deref() == Some(server_id) {
            return Err(FakeError);
        }
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        rows.retain(|row| row.server_id != server_id);
        drop(rows);
        self.cleared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(server_id.to_owned());
        Ok(())
    }
}

#[derive(Default)]
struct CapturedLog {
    flow_events: Mutex<Vec<String>>,
    issuer_events: Mutex<Vec<String>>,
}

impl CapturedLog {
    fn flow_events(&self) -> Vec<String> {
        self.flow_events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn issuer_events(&self) -> Vec<String> {
        self.issuer_events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BackfillLog for CapturedLog {
    fn event(&self, event: BackfillEvent<'_>) {
        let (from_flow, line) = match event {
            BackfillEvent::FlowAmbiguous { server_id } => (true, format!("ambiguous:{server_id}")),
            BackfillEvent::FlowStamped {
                server_id,
                flow,
                rule,
            } => (
                true,
                format!("stamped:{server_id}:{}:{}", flow_name(flow), rule.as_str()),
            ),
            BackfillEvent::FlowProcessed { rows, counts } => {
                (true, format!("processed:{rows}:{}", counts.len()))
            }
            BackfillEvent::IssuerHealFailed { server_id, error } => {
                (false, format!("heal_failed:{server_id}:{error}"))
            }
            BackfillEvent::IssuerHealed {
                server_id,
                issuer,
                alias,
            } => (
                false,
                format!("healed:{server_id}:{issuer}:{}", alias.unwrap_or_default()),
            ),
            BackfillEvent::IssuerHealedCount { healed } => {
                (false, format!("healed_count:{healed}"))
            }
        };
        let events = if from_flow {
            &self.flow_events
        } else {
            &self.issuer_events
        };
        events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
    }
}

#[derive(Clone, Default)]
struct CapturedBytes(Arc<Mutex<Vec<u8>>>);

impl CapturedBytes {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the captured log lines are UTF-8")
    }
}

impl io::Write for CapturedBytes {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for CapturedBytes {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn issuer_row(server_id: &str) -> IssuerCandidateRow {
    IssuerCandidateRow {
        server_id: server_id.to_owned(),
        alias: None,
        server_name: Some(format!("Name {server_id}")),
        auth_type: Some(MCPAuth::Oauth2),
        issuer: Some("https://idp.example.com".to_owned()),
        authorization_url: Some("https://idp.example.com/authorize".to_owned()),
        token_url: Some("https://idp.example.com/token".to_owned()),
        registration_url: None,
        updated_by: Some(DISCOVERY_ACTOR.to_owned()),
    }
}

#[test]
fn flow_classification_prefers_per_user_tokens() {
    let blob = credential_fields(Some("id"), Some("secret"));
    let classification = classify_null_flow_row(&FlowSignals {
        has_per_user_tokens: true,
        authorization_url: Some("https://idp/authorize"),
        registration_url: Some("https://idp/register"),
        token_url: Some("https://idp/token"),
        credentials: Some(&blob),
    });
    assert_eq!(classification.flow, Some(OAuth2Flow::AuthorizationCode));
    assert_eq!(classification.rule, BackfillRule::PerUserTokens);
}

#[test]
fn flow_classification_uses_the_authorization_url() {
    let blob = credential_fields(Some("id"), Some("secret"));
    let classification = classify_null_flow_row(&signals(
        Some("https://idp/authorize"),
        Some("https://idp/register"),
        Some("https://idp/token"),
        Some(&blob),
    ));
    assert_eq!(classification.flow, Some(OAuth2Flow::AuthorizationCode));
    assert_eq!(classification.rule, BackfillRule::AuthorizationUrl);
}

#[test]
fn flow_classification_uses_the_registration_url() {
    let blob = credential_fields(Some("id"), Some("secret"));
    let classification = classify_null_flow_row(&signals(
        None,
        Some("https://idp/register"),
        Some("https://idp/token"),
        Some(&blob),
    ));
    assert_eq!(classification.flow, Some(OAuth2Flow::AuthorizationCode));
    assert_eq!(classification.rule, BackfillRule::RegistrationUrl);
}

#[test]
fn flow_classification_leaves_the_ambiguous_m2m_shape_unstamped() {
    let blob = credential_fields(Some("id"), Some("secret"));
    let classification =
        classify_null_flow_row(&signals(None, None, Some("https://idp/token"), Some(&blob)));
    assert_eq!(classification.flow, None);
    assert_eq!(classification.rule, BackfillRule::AmbiguousM2mShape);
}

#[test]
fn flow_classification_defaults_to_interactive() {
    let classification = classify_null_flow_row(&signals(None, None, None, None));
    assert_eq!(classification.flow, Some(OAuth2Flow::AuthorizationCode));
    assert_eq!(classification.rule, BackfillRule::InteractiveDefault);
}

#[test]
fn flow_classification_ignores_empty_endpoints_and_partial_credentials() {
    let blob = credential_fields(Some("id"), Some("secret"));
    let empty_authorization =
        classify_null_flow_row(&signals(Some(""), Some("https://idp/register"), None, None));
    assert_eq!(empty_authorization.rule, BackfillRule::RegistrationUrl);
    let empty_registration = classify_null_flow_row(&signals(Some(""), Some(""), None, None));
    assert_eq!(empty_registration.rule, BackfillRule::InteractiveDefault);
    let partial = credential_fields(Some("id"), None);
    let partial_credentials = classify_null_flow_row(&signals(
        None,
        None,
        Some("https://idp/token"),
        Some(&partial),
    ));
    assert_eq!(partial_credentials.rule, BackfillRule::InteractiveDefault);
    let empty_token_url = classify_null_flow_row(&signals(None, None, Some(""), Some(&blob)));
    assert_eq!(empty_token_url.rule, BackfillRule::InteractiveDefault);
}

#[test]
fn flow_classification_ignores_empty_credential_strings() {
    let blob = credential_fields(Some("id"), Some(""));
    let classification =
        classify_null_flow_row(&signals(None, None, Some("https://idp/token"), Some(&blob)));
    assert_eq!(classification.flow, Some(OAuth2Flow::AuthorizationCode));
    assert_eq!(classification.rule, BackfillRule::InteractiveDefault);
}

#[test]
fn flow_and_rule_names_match_litellm() {
    assert_eq!(BackfillRule::PerUserTokens.as_str(), "per_user_tokens");
    assert_eq!(BackfillRule::AuthorizationUrl.as_str(), "authorization_url");
    assert_eq!(BackfillRule::RegistrationUrl.as_str(), "registration_url");
    assert_eq!(
        BackfillRule::AmbiguousM2mShape.as_str(),
        "ambiguous_m2m_shape"
    );
    assert_eq!(
        BackfillRule::InteractiveDefault.as_str(),
        "interactive_default"
    );
    assert_eq!(
        flow_name(OAuth2Flow::ClientCredentials),
        "client_credentials"
    );
    assert_eq!(
        flow_name(OAuth2Flow::AuthorizationCode),
        "authorization_code"
    );
}

#[test]
fn flow_decrypted_credentials_reads_an_object_and_an_encoded_string() {
    let cipher = test_cipher();
    let object = credentials_value(Some("id"), Some("secret"));
    let from_object = decrypted_credentials(object.as_ref(), &cipher).expect("the blob reads");
    assert_eq!(
        from_object.get("client_id"),
        Some(&JsonValue::String("id".to_owned()))
    );
    let encoded = JsonValue::String(r#"{"client_id":"id","client_secret":"secret"}"#.to_owned());
    let from_string = decrypted_credentials(Some(&encoded), &cipher).expect("the blob reads");
    assert_eq!(
        from_string.get("client_secret"),
        Some(&JsonValue::String("secret".to_owned()))
    );
}

#[test]
fn flow_decrypted_credentials_returns_none_for_unreadable_input() {
    let cipher = test_cipher();
    assert!(decrypted_credentials(None, &cipher).is_none());
    let array = JsonValue::Array(Vec::new());
    assert!(decrypted_credentials(Some(&array), &cipher).is_none());
    let broken = JsonValue::String("not json".to_owned());
    assert!(decrypted_credentials(Some(&broken), &cipher).is_none());
}

#[test]
fn flow_decrypted_credentials_opens_an_encrypted_secret() {
    let cipher = test_cipher();
    let sealed = cipher
        .encrypt_value(
            StoredValue::<()>::Text(SecretString::from("plain-secret".to_owned())),
            None,
        )
        .expect("the cipher seals the value");
    let StoredValue::Text(sealed_text) = sealed else {
        panic!("the sealed value stays text");
    };
    let mut fields = BTreeMap::new();
    fields.insert(
        "client_secret".to_owned(),
        JsonValue::String(sealed_text.expose_secret().to_owned()),
    );
    let blob = decrypted_credentials(Some(&JsonValue::Object(fields)), &cipher).expect("the blob");
    assert_eq!(
        blob.get("client_secret"),
        Some(&JsonValue::String("plain-secret".to_owned()))
    );
}

fn five_rule_table() -> FakeFlowTable {
    let mut per_user = flow_row("srv-per-user");
    per_user.token_url = Some("https://idp/token".to_owned());
    let mut authorization = flow_row("srv-authorization");
    authorization.authorization_url = Some("https://idp/authorize".to_owned());
    authorization.registration_url = Some("https://idp/register".to_owned());
    authorization.credentials = credentials_value(Some("id"), Some("secret"));
    let mut registration = flow_row("srv-registration");
    registration.registration_url = Some("https://idp/register".to_owned());
    registration.token_url = Some("https://idp/token".to_owned());
    registration.credentials = credentials_value(Some("id"), Some("secret"));
    let mut ambiguous = flow_row("srv-ambiguous");
    ambiguous.token_url = Some("https://idp/token".to_owned());
    ambiguous.credentials = credentials_value(Some("id"), Some("secret"));
    let tokens = vec![UserCredentialRow {
        server_id: "srv-per-user".to_owned(),
        credential_b64: oauth_payload_b64(),
    }];
    FakeFlowTable::new(
        vec![
            per_user,
            authorization,
            registration,
            ambiguous,
            flow_row("srv-default"),
        ],
        tokens,
    )
}

#[tokio::test]
async fn flow_backfill_stamps_provable_rows_in_one_batch() {
    let table = five_rule_table();
    let log = CapturedLog::default();
    let counts = backfill_null_oauth2_flows(&table, &test_cipher(), &log)
        .await
        .expect("the backfill runs");
    assert_eq!(table.stamps().len(), 1);
    let (server_ids, flow) = table.stamps().first().cloned().expect("one batch");
    assert_eq!(flow, OAuth2Flow::AuthorizationCode);
    assert_eq!(
        server_ids,
        vec![
            "srv-per-user".to_owned(),
            "srv-authorization".to_owned(),
            "srv-registration".to_owned(),
            "srv-default".to_owned(),
        ]
    );
    assert_eq!(counts.len(), 5);
}

#[tokio::test]
async fn flow_backfill_counts_each_rule() {
    let table = five_rule_table();
    let log = CapturedLog::default();
    let counts = backfill_null_oauth2_flows(&table, &test_cipher(), &log)
        .await
        .expect("the backfill runs");
    let expected = BTreeMap::from([
        (BackfillRule::PerUserTokens, 1),
        (BackfillRule::AuthorizationUrl, 1),
        (BackfillRule::RegistrationUrl, 1),
        (BackfillRule::AmbiguousM2mShape, 1),
        (BackfillRule::InteractiveDefault, 1),
    ]);
    assert_eq!(counts, expected);
    assert!(log.flow_events().contains(&"processed:5:5".to_owned()));
}

#[tokio::test]
async fn flow_backfill_warns_on_ambiguous_rows() {
    let table = five_rule_table();
    let log = CapturedLog::default();
    backfill_null_oauth2_flows(&table, &test_cipher(), &log)
        .await
        .expect("the backfill runs");
    let ambiguous: Vec<String> = log
        .flow_events()
        .into_iter()
        .filter(|line| line.starts_with("ambiguous:"))
        .collect();
    assert_eq!(ambiguous, vec!["ambiguous:srv-ambiguous".to_owned()]);
}

#[tokio::test]
async fn flow_backfill_never_stamps_client_credentials() {
    let mut ambiguous = flow_row("srv-m2m");
    ambiguous.token_url = Some("https://idp/token".to_owned());
    ambiguous.credentials = credentials_value(Some("id"), Some("secret"));
    let table = FakeFlowTable::new(vec![ambiguous], Vec::new());
    let log = CapturedLog::default();
    let counts = backfill_null_oauth2_flows(&table, &test_cipher(), &log)
        .await
        .expect("the backfill runs");
    assert!(table.stamps().is_empty());
    assert_eq!(counts.get(&BackfillRule::AmbiguousM2mShape), Some(&1));
}

#[tokio::test]
async fn flow_backfill_returns_nothing_for_an_empty_table() {
    let table = FakeFlowTable::new(Vec::new(), Vec::new());
    let log = CapturedLog::default();
    let counts = backfill_null_oauth2_flows(&table, &test_cipher(), &log)
        .await
        .expect("the backfill runs");
    assert!(counts.is_empty());
    assert!(table.stamps().is_empty());
    assert!(log.flow_events().is_empty());
}

#[rstest]
#[case::ambiguous_server("server is ambiguous")]
#[case::stamped_row("stamped row")]
#[case::flow("authorization_code")]
#[case::rule("per_user_tokens")]
#[case::processed("processed oauth2 server rows")]
#[case::heal_failed("could not heal server")]
#[case::alias("idp-alias")]
#[case::healed("the configured endpoints apply again")]
#[case::heal_count("healed servers")]
fn tracing_log_names_every_backfill_event(#[case] needle: &str) {
    let captured = CapturedBytes::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer(captured.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let log = TracingBackfillLog;
    let counts: BTreeMap<BackfillRule, usize> = BTreeMap::from([(BackfillRule::PerUserTokens, 1)]);
    log.event(BackfillEvent::FlowAmbiguous { server_id: "srv-1" });
    log.event(BackfillEvent::FlowStamped {
        server_id: "srv-1",
        flow: OAuth2Flow::AuthorizationCode,
        rule: BackfillRule::PerUserTokens,
    });
    log.event(BackfillEvent::FlowProcessed {
        rows: 3,
        counts: &counts,
    });
    log.event(BackfillEvent::IssuerHealFailed {
        server_id: "srv-1",
        error: "boom",
    });
    log.event(BackfillEvent::IssuerHealed {
        server_id: "srv-1",
        issuer: "https://idp.example.com",
        alias: Some("idp-alias"),
    });
    log.event(BackfillEvent::IssuerHealedCount { healed: 2 });
    let text = captured.contents();
    assert!(text.contains(needle), "missing {needle} in {text}");
}

#[test]
fn issuer_predicate_matches_a_discovery_stamped_row() {
    assert!(is_stamped_issuer_row(&issuer_row("srv-1")));
    let mut trailing = issuer_row("srv-1");
    trailing.issuer = Some("  https://idp.example.com  ".to_owned());
    assert!(is_stamped_issuer_row(&trailing));
    let mut default_port = issuer_row("srv-1");
    default_port.token_url = Some("https://idp.example.com:443/token".to_owned());
    assert!(is_stamped_issuer_row(&default_port));
}

#[test]
fn issuer_predicate_requires_the_discovery_actor() {
    let mut foreign = issuer_row("srv-1");
    foreign.updated_by = Some("oauth2_flow_backfill".to_owned());
    assert!(!is_stamped_issuer_row(&foreign));
    let mut missing = issuer_row("srv-1");
    missing.updated_by = None;
    assert!(!is_stamped_issuer_row(&missing));
}

#[test]
fn issuer_predicate_requires_an_issuer() {
    let mut blank = issuer_row("srv-1");
    blank.issuer = Some("   ".to_owned());
    assert!(!is_stamped_issuer_row(&blank));
    let mut missing = issuer_row("srv-1");
    missing.issuer = None;
    assert!(!is_stamped_issuer_row(&missing));
}

#[rstest]
#[case::oauth2(MCPAuth::Oauth2)]
#[case::true_passthrough(MCPAuth::TruePassthrough)]
#[case::oauth_delegate(MCPAuth::OauthDelegate)]
fn issuer_predicate_requires_an_anchoring_auth_type(#[case] auth_type: MCPAuth) {
    let mut row = issuer_row("srv-1");
    row.auth_type = Some(auth_type);
    assert!(is_stamped_issuer_row(&row), "for {auth_type:?}");
}

#[test]
fn issuer_predicate_rejects_other_auth_types() {
    let mut foreign = issuer_row("srv-1");
    foreign.auth_type = Some(MCPAuth::ApiKey);
    assert!(!is_stamped_issuer_row(&foreign));
    let mut missing = issuer_row("srv-1");
    missing.auth_type = None;
    assert!(!is_stamped_issuer_row(&missing));
}

#[test]
fn issuer_predicate_requires_a_configured_endpoint() {
    let mut missing = issuer_row("srv-1");
    missing.authorization_url = None;
    missing.token_url = None;
    missing.registration_url = None;
    assert!(!is_stamped_issuer_row(&missing));
    let mut blank = issuer_row("srv-1");
    blank.authorization_url = Some(String::new());
    blank.token_url = Some("   ".to_owned());
    blank.registration_url = None;
    assert!(!is_stamped_issuer_row(&blank));
}

#[test]
fn issuer_predicate_requires_matching_origins() {
    let mut other_origin = issuer_row("srv-1");
    other_origin.token_url = Some("https://other.example.com/token".to_owned());
    assert!(!is_stamped_issuer_row(&other_origin));
    let mut other_scheme = issuer_row("srv-1");
    other_scheme.authorization_url = Some("http://idp.example.com/authorize".to_owned());
    assert!(!is_stamped_issuer_row(&other_scheme));
    let mut malformed_endpoint = issuer_row("srv-1");
    malformed_endpoint.registration_url = Some("not a url".to_owned());
    assert!(!is_stamped_issuer_row(&malformed_endpoint));
    let mut malformed_issuer = issuer_row("srv-1");
    malformed_issuer.issuer = Some("not a url".to_owned());
    assert!(!is_stamped_issuer_row(&malformed_issuer));
}

#[test]
fn origin_needs_both_a_scheme_and_a_host() {
    assert_eq!(
        origin("https://idp.example.com:443/token"),
        Some("https://idp.example.com".to_owned())
    );
    assert_eq!(origin("//idp.example.com/token"), None);
    assert_eq!(origin("https:/token"), None);
}

#[tokio::test]
async fn issuer_backfill_heals_only_matching_rows() {
    let mut mismatch = issuer_row("srv-2");
    mismatch.token_url = Some("https://other.example.com/token".to_owned());
    let table = FakeIssuerTable::new(vec![issuer_row("srv-1"), mismatch], None);
    let log = CapturedLog::default();
    let healed = heal_stamped_issuers(&table, &log)
        .await
        .expect("the backfill runs");
    assert_eq!(healed, 1);
    assert_eq!(table.cleared(), vec!["srv-1".to_owned()]);
    assert!(log.issuer_events().contains(&"healed_count:1".to_owned()));
    assert!(
        !log.issuer_events()
            .iter()
            .any(|line| line.starts_with("heal_failed:"))
    );
}

#[tokio::test]
async fn issuer_backfill_reports_the_alias_or_the_server_name() {
    let mut aliased = issuer_row("srv-1");
    aliased.alias = Some("idp-alias".to_owned());
    let table = FakeIssuerTable::new(vec![aliased, issuer_row("srv-2")], None);
    let log = CapturedLog::default();
    let healed = heal_stamped_issuers(&table, &log)
        .await
        .expect("the backfill runs");
    assert_eq!(healed, 2);
    let healed_lines: Vec<String> = log
        .issuer_events()
        .into_iter()
        .filter(|line| line.starts_with("healed:"))
        .collect();
    assert_eq!(
        healed_lines,
        vec![
            "healed:srv-1:https://idp.example.com:idp-alias".to_owned(),
            "healed:srv-2:https://idp.example.com:Name srv-2".to_owned(),
        ]
    );
}

#[tokio::test]
async fn issuer_backfill_continues_after_a_failed_row() {
    let table = FakeIssuerTable::new(
        vec![issuer_row("srv-1"), issuer_row("srv-2")],
        Some("srv-1"),
    );
    let log = CapturedLog::default();
    let healed = heal_stamped_issuers(&table, &log)
        .await
        .expect("the backfill runs");
    assert_eq!(healed, 1);
    assert_eq!(table.cleared(), vec!["srv-2".to_owned()]);
    assert!(
        log.issuer_events()
            .iter()
            .any(|line| line.starts_with("heal_failed:srv-1:"))
    );
    assert!(log.issuer_events().contains(&"healed_count:1".to_owned()));
}

#[tokio::test]
async fn issuer_backfill_is_idempotent() {
    let table = FakeIssuerTable::new(vec![issuer_row("srv-1")], None);
    let first = CapturedLog::default();
    assert_eq!(
        heal_stamped_issuers(&table, &first)
            .await
            .expect("the first run"),
        1
    );
    let second = CapturedLog::default();
    assert_eq!(
        heal_stamped_issuers(&table, &second)
            .await
            .expect("the second run"),
        0
    );
    assert_eq!(table.cleared(), vec!["srv-1".to_owned()]);
    assert!(second.issuer_events().is_empty());
}

#[tokio::test]
async fn issuer_backfill_reports_zero_without_candidates() {
    let mut mismatch = issuer_row("srv-1");
    mismatch.updated_by = Some("oauth2_flow_backfill".to_owned());
    let table = FakeIssuerTable::new(vec![mismatch], None);
    let log = CapturedLog::default();
    assert_eq!(
        heal_stamped_issuers(&table, &log)
            .await
            .expect("the backfill runs"),
        0
    );
    assert!(table.cleared().is_empty());
    assert!(log.issuer_events().is_empty());
}
