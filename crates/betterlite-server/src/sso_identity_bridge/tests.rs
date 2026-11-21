#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::identity::{AgentIdentityFailure, FailureCode, MicrosoftInteractiveSubject};
use betterlite_config::sso_identity_bridge::{
    GENERIC_CLIENT_ID_ENV, GOOGLE_CLIENT_ID_ENV, MICROSOFT_CLIENT_ID_ENV,
    SAML_IDP_METADATA_URL_ENV, SAML_IDP_METADATA_XML_ENV, SsoIdentityBridgeSettings,
    load_bridge_settings,
};
use betterlite_mcp::sso_assertions::assertion::SsoIdentityAssertion;
use rstest::rstest;
use secrecy::SecretString;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::enrollment::{InteractiveEnroller, enroll_microsoft_subject};
use super::microsoft_subject::{MicrosoftEndpoints, microsoft_interactive_subject};
use super::provider::{
    ActiveSsoProvider, active_sso_provider, id_jag_assertion_capture_gap,
    id_jag_assertion_capture_gap_at_startup,
};
use super::warnings::{
    RetentionCheck, warn_if_id_jag_assertion_uncaptured, warn_if_id_jag_capture_gap,
};

const TENANT: &str = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
const OBJECT_ID: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
const NONE_GAP: &str = "no SSO provider is configured, so no IdP identity assertion is ever captured and ID-JAG credential resolution fails for every user. Point SSO at the generic OIDC provider (GENERIC_CLIENT_ID), the one login path whose token response the gateway retains an id_token from";
const UNCAPTURED_GAP: &str = "has no identity-assertion capture path, so no IdP id_token is ever stored and ID-JAG credential resolution fails for every user no matter how often they sign in. Point SSO at the generic OIDC provider (GENERIC_CLIENT_ID), the one login path whose token response the gateway retains an id_token from";
const UNCAPTURED_WARNING: &str =
    "SSO login captured no IdP identity assertion while an oauth2_id_jag MCP server is registered";
const NO_ID_TOKEN_DETAIL: &str =
    "the identity provider's token response carried no usable id_token";
const DEBUG_CALLBACK_WARNING: &str = "SSO debug callback ran with an oauth2_id_jag capture gap";

fn settings(pairs: &[(&str, &str)]) -> SsoIdentityBridgeSettings {
    let environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect();
    load_bridge_settings(&environment)
}

fn expectation(provider: &str) -> String {
    format!("the active SSO provider ({provider}) {UNCAPTURED_GAP}")
}

#[derive(Clone, Debug)]
struct RetentionFailure;

impl fmt::Display for RetentionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the retention query is unavailable")
    }
}

impl StdError for RetentionFailure {}

struct FakeRetention {
    enabled: Result<bool, RetentionFailure>,
    calls: AtomicUsize,
}

impl FakeRetention {
    fn new(enabled: Result<bool, RetentionFailure>) -> Self {
        Self {
            enabled,
            calls: AtomicUsize::new(0),
        }
    }
}

impl RetentionCheck for FakeRetention {
    type Error = RetentionFailure;

    fn retention_enabled(&self) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let enabled = self.enabled.clone();
        async move { enabled }
    }
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the captured log lines are UTF-8")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

#[rstest]
#[case::nothing_configured(&[], ActiveSsoProvider::None)]
#[case::google_alone(&[(GOOGLE_CLIENT_ID_ENV, "google-id")], ActiveSsoProvider::Google)]
#[case::microsoft_alone(&[(MICROSOFT_CLIENT_ID_ENV, "microsoft-id")], ActiveSsoProvider::Microsoft)]
#[case::generic_alone(&[(GENERIC_CLIENT_ID_ENV, "generic-id")], ActiveSsoProvider::Generic)]
#[case::saml_url(&[(SAML_IDP_METADATA_URL_ENV, "https://idp.example/metadata")], ActiveSsoProvider::Saml)]
#[case::saml_xml(&[(SAML_IDP_METADATA_XML_ENV, "<xml/>")], ActiveSsoProvider::Saml)]
#[case::empty_google_beats_generic(&[(GOOGLE_CLIENT_ID_ENV, ""), (GENERIC_CLIENT_ID_ENV, "generic-id")], ActiveSsoProvider::Google)]
#[case::empty_microsoft_beats_generic(&[(MICROSOFT_CLIENT_ID_ENV, ""), (GENERIC_CLIENT_ID_ENV, "generic-id")], ActiveSsoProvider::Microsoft)]
#[case::empty_generic_beats_saml(&[(GENERIC_CLIENT_ID_ENV, ""), (SAML_IDP_METADATA_URL_ENV, "https://idp.example/metadata")], ActiveSsoProvider::Generic)]
#[case::google_beats_microsoft(&[(GOOGLE_CLIENT_ID_ENV, "google-id"), (MICROSOFT_CLIENT_ID_ENV, "microsoft-id")], ActiveSsoProvider::Google)]
#[case::microsoft_beats_generic(&[(MICROSOFT_CLIENT_ID_ENV, "microsoft-id"), (GENERIC_CLIENT_ID_ENV, "generic-id")], ActiveSsoProvider::Microsoft)]
#[case::empty_saml_url_alone(&[(SAML_IDP_METADATA_URL_ENV, "")], ActiveSsoProvider::None)]
#[case::empty_saml_url_with_xml(&[(SAML_IDP_METADATA_URL_ENV, ""), (SAML_IDP_METADATA_XML_ENV, "<xml/>")], ActiveSsoProvider::Saml)]
#[case::whitespace_saml_url(&[(SAML_IDP_METADATA_URL_ENV, " ")], ActiveSsoProvider::Saml)]
fn provider_precedence_follows_the_callback(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: ActiveSsoProvider,
) {
    assert_eq!(active_sso_provider(&settings(pairs)), expected);
}

#[rstest]
#[case::google(ActiveSsoProvider::Google, "google")]
#[case::microsoft(ActiveSsoProvider::Microsoft, "microsoft")]
#[case::generic(ActiveSsoProvider::Generic, "generic")]
#[case::saml(ActiveSsoProvider::Saml, "saml")]
#[case::none(ActiveSsoProvider::None, "none")]
fn provider_values_match_litellm(#[case] provider: ActiveSsoProvider, #[case] expected: &str) {
    assert_eq!(provider.value(), expected);
}

#[test]
fn generic_capture_has_no_gap() {
    assert_eq!(
        id_jag_assertion_capture_gap(&settings(&[(GENERIC_CLIENT_ID_ENV, "generic-id")])),
        None
    );
}

#[test]
fn no_provider_gap_names_the_missing_configuration() {
    assert_eq!(
        id_jag_assertion_capture_gap(&settings(&[])),
        Some(String::from(NONE_GAP))
    );
}

#[rstest]
#[case::google(GOOGLE_CLIENT_ID_ENV, "google-id", "google")]
#[case::microsoft(MICROSOFT_CLIENT_ID_ENV, "microsoft-id", "microsoft")]
#[case::saml(SAML_IDP_METADATA_URL_ENV, "https://idp.example/metadata", "saml")]
fn uncapturable_provider_gap_names_the_provider(
    #[case] name: &str,
    #[case] value: &str,
    #[case] provider: &str,
) {
    assert_eq!(
        id_jag_assertion_capture_gap(&settings(&[(name, value)])),
        Some(expectation(provider))
    );
}

#[test]
fn startup_gap_is_none_without_a_provider_where_the_login_gap_reports_one() {
    let configured = settings(&[]);
    assert_eq!(id_jag_assertion_capture_gap_at_startup(&configured), None);
    assert_eq!(
        id_jag_assertion_capture_gap(&configured),
        Some(String::from(NONE_GAP))
    );
}

#[rstest]
#[case::google(&[(GOOGLE_CLIENT_ID_ENV, "google-id")])]
#[case::microsoft(&[(MICROSOFT_CLIENT_ID_ENV, "microsoft-id")])]
#[case::generic(&[(GENERIC_CLIENT_ID_ENV, "generic-id")])]
#[case::saml(&[(SAML_IDP_METADATA_XML_ENV, "<xml/>")])]
fn startup_gap_is_the_login_gap_with_an_active_provider(#[case] pairs: &[(&str, &str)]) {
    let configured = settings(pairs);
    assert_eq!(
        id_jag_assertion_capture_gap_at_startup(&configured),
        id_jag_assertion_capture_gap(&configured)
    );
}

#[tokio::test]
async fn a_supplied_assertion_never_reaches_the_retention_gate() {
    let (logs, guard) = capture_logs();
    let retention = FakeRetention::new(Ok(true));
    let assertion = SsoIdentityAssertion {
        id_token: SecretString::from("id-token"),
        refresh_token: None,
        issuer: None,
        expires_at: None,
    };
    warn_if_id_jag_assertion_uncaptured(&settings(&[]), Some(&assertion), &retention).await;
    drop(guard);
    assert_eq!(retention.calls.load(Ordering::Relaxed), 0);
    assert!(!logs.contents().contains(UNCAPTURED_WARNING));
}

#[tokio::test]
async fn a_disabled_retention_gate_suppresses_the_uncaptured_warning() {
    let (logs, guard) = capture_logs();
    let retention = FakeRetention::new(Ok(false));
    warn_if_id_jag_assertion_uncaptured(&settings(&[]), None, &retention).await;
    drop(guard);
    assert_eq!(retention.calls.load(Ordering::Relaxed), 1);
    assert!(!logs.contents().contains(UNCAPTURED_WARNING));
}

#[tokio::test]
async fn a_retention_outage_is_a_debug_line_and_the_login_continues() {
    let (logs, guard) = capture_logs();
    let retention = FakeRetention::new(Err(RetentionFailure));
    warn_if_id_jag_assertion_uncaptured(&settings(&[]), None, &retention).await;
    drop(guard);
    let captured = logs.contents();
    assert!(captured.contains(
        "Could not check for oauth2_id_jag MCP servers after SSO login: the retention query is unavailable"
    ));
    assert!(!captured.contains(UNCAPTURED_WARNING));
}

#[tokio::test]
async fn an_enabled_retention_gate_warns_with_the_provider_gap() {
    let (logs, guard) = capture_logs();
    let retention = FakeRetention::new(Ok(true));
    let configured = settings(&[(GOOGLE_CLIENT_ID_ENV, "google-id")]);
    warn_if_id_jag_assertion_uncaptured(&configured, None, &retention).await;
    drop(guard);
    assert!(
        logs.contents()
            .contains(&format!("{UNCAPTURED_WARNING}: {}", expectation("google")))
    );
}

#[tokio::test]
async fn generic_capture_warns_with_the_missing_id_token_detail() {
    let (logs, guard) = capture_logs();
    let retention = FakeRetention::new(Ok(true));
    let configured = settings(&[(GENERIC_CLIENT_ID_ENV, "generic-id")]);
    warn_if_id_jag_assertion_uncaptured(&configured, None, &retention).await;
    drop(guard);
    assert!(
        logs.contents()
            .contains(&format!("{UNCAPTURED_WARNING}: {NO_ID_TOKEN_DETAIL}"))
    );
}

#[tokio::test]
async fn the_capture_gap_warning_skips_the_retention_gate_for_generic() {
    let (logs, guard) = capture_logs();
    let retention = FakeRetention::new(Ok(true));
    warn_if_id_jag_capture_gap(
        &settings(&[(GENERIC_CLIENT_ID_ENV, "generic-id")]),
        &retention,
    )
    .await;
    drop(guard);
    assert_eq!(retention.calls.load(Ordering::Relaxed), 0);
    assert!(!logs.contents().contains(DEBUG_CALLBACK_WARNING));
}

#[tokio::test]
async fn the_capture_gap_warning_swallows_a_retention_outage() {
    let (logs, guard) = capture_logs();
    let retention = FakeRetention::new(Err(RetentionFailure));
    warn_if_id_jag_capture_gap(&settings(&[]), &retention).await;
    drop(guard);
    let captured = logs.contents();
    assert!(captured.contains(
        "Could not check for oauth2_id_jag MCP servers: the retention query is unavailable"
    ));
    assert!(!captured.contains(DEBUG_CALLBACK_WARNING));
}

#[tokio::test]
async fn the_capture_gap_warning_requires_an_enabled_gate() {
    let (logs, guard) = capture_logs();
    let retention = FakeRetention::new(Ok(false));
    warn_if_id_jag_capture_gap(&settings(&[]), &retention).await;
    drop(guard);
    assert_eq!(retention.calls.load(Ordering::Relaxed), 1);
    assert!(!logs.contents().contains(DEBUG_CALLBACK_WARNING));
}

#[tokio::test]
async fn the_capture_gap_warning_names_the_gap() {
    let (logs, guard) = capture_logs();
    let retention = FakeRetention::new(Ok(true));
    let configured = settings(&[(SAML_IDP_METADATA_XML_ENV, "<xml/>")]);
    warn_if_id_jag_capture_gap(&configured, &retention).await;
    drop(guard);
    assert!(logs.contents().contains(&format!(
        "{DEBUG_CALLBACK_WARNING}: {}",
        expectation("saml")
    )));
}

#[rstest]
#[case::missing_tenant(None, Some(OBJECT_ID))]
#[case::unparseable_tenant(Some("not-a-uuid"), Some(OBJECT_ID))]
#[case::missing_response_id(Some(TENANT), None)]
#[case::unparseable_response_id(Some(TENANT), Some("not-a-uuid"))]
fn projection_needs_parseable_uuids(
    #[case] tenant: Option<&str>,
    #[case] response_id: Option<&str>,
) {
    assert!(
        microsoft_interactive_subject(tenant, response_id, &MicrosoftEndpoints::default())
            .is_none()
    );
}

#[test]
fn projection_canonicalizes_tenant_and_object_ids() {
    let subject = microsoft_interactive_subject(
        Some("{6F9619FF-8B86-D011-B42D-00C04FC964FF}"),
        Some("AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE"),
        &MicrosoftEndpoints::default(),
    )
    .expect("the canonical subject");
    assert_eq!(
        subject.issuer,
        "https://login.microsoftonline.com/6f9619ff-8b86-d011-b42d-00c04fc964ff/v2.0"
    );
    assert_eq!(subject.tenant_id, TENANT);
    assert_eq!(subject.oid, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
}

#[test]
fn projection_accepts_absent_and_empty_endpoint_overrides() {
    let endpoints = MicrosoftEndpoints {
        authorization: Some(""),
        token: None,
        userinfo: Some(""),
    };
    assert!(microsoft_interactive_subject(Some(TENANT), Some(OBJECT_ID), &endpoints).is_some());
}

#[test]
fn projection_accepts_the_exact_canonical_endpoints() {
    let endpoints = MicrosoftEndpoints {
        authorization: Some(
            "https://login.microsoftonline.com/6f9619ff-8b86-d011-b42d-00c04fc964ff/oauth2/v2.0/authorize",
        ),
        token: Some(
            "https://login.microsoftonline.com/6f9619ff-8b86-d011-b42d-00c04fc964ff/oauth2/v2.0/token",
        ),
        userinfo: Some("https://graph.microsoft.com/v1.0/me"),
    };
    assert!(microsoft_interactive_subject(Some(TENANT), Some(OBJECT_ID), &endpoints).is_some());
}

#[rstest]
#[case::sovereign_authorization(
    Some(
        "https://login.microsoftonline.us/6f9619ff-8b86-d011-b42d-00c04fc964ff/oauth2/v2.0/authorize"
    ),
    None,
    None
)]
#[case::foreign_token(
    Some(""),
    Some("https://login.microsoftonline.com/other-tenant/oauth2/v2.0/token"),
    None
)]
#[case::sovereign_userinfo(None, None, Some("https://graph.microsoft.us/v1.0/me"))]
#[case::trailing_slash(
    Some(
        "https://login.microsoftonline.com/6f9619ff-8b86-d011-b42d-00c04fc964ff/oauth2/v2.0/authorize/"
    ),
    None,
    None
)]
fn projection_rejects_foreign_or_noncanonical_endpoints(
    #[case] authorization: Option<&str>,
    #[case] token: Option<&str>,
    #[case] userinfo: Option<&str>,
) {
    let endpoints = MicrosoftEndpoints {
        authorization,
        token,
        userinfo,
    };
    assert!(microsoft_interactive_subject(Some(TENANT), Some(OBJECT_ID), &endpoints).is_none());
}

fn microsoft_subject() -> MicrosoftInteractiveSubject {
    MicrosoftInteractiveSubject {
        issuer: String::from(
            "https://login.microsoftonline.com/6f9619ff-8b86-d011-b42d-00c04fc964ff/v2.0",
        ),
        tenant_id: String::from(TENANT),
        oid: String::from(OBJECT_ID),
    }
}

#[derive(Default)]
struct FakeEnroller {
    calls: AtomicUsize,
    recorded: Mutex<Option<(MicrosoftInteractiveSubject, String)>>,
    failure: Option<AgentIdentityFailure>,
}

impl InteractiveEnroller for FakeEnroller {
    fn enroll_interactive_human(
        &self,
        subject: &MicrosoftInteractiveSubject,
        user_id: &str,
    ) -> impl Future<Output = Result<(), AgentIdentityFailure>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.recorded.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((subject.clone(), user_id.to_owned()));
        let failure = self.failure.clone();
        async move {
            match failure {
                Some(failure) => Err(failure),
                None => Ok(()),
            }
        }
    }
}

#[tokio::test]
async fn enrollment_ignores_a_missing_subject_or_user_id() {
    let store = FakeEnroller::default();
    let subject = microsoft_subject();
    enroll_microsoft_subject(None, Some("user-1"), &store)
        .await
        .unwrap();
    enroll_microsoft_subject(Some(&subject), None, &store)
        .await
        .unwrap();
    enroll_microsoft_subject(Some(&subject), Some(""), &store)
        .await
        .unwrap();
    assert_eq!(store.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn enrollment_reaches_the_identity_store_with_the_subject_and_user() {
    let store = FakeEnroller::default();
    let subject = microsoft_subject();
    enroll_microsoft_subject(Some(&subject), Some("user-1"), &store)
        .await
        .unwrap();
    assert_eq!(store.calls.load(Ordering::Relaxed), 1);
    let recorded = store
        .recorded
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .expect("the recorded enrollment");
    assert_eq!(recorded.0.issuer, subject.issuer);
    assert_eq!(recorded.0.tenant_id, subject.tenant_id);
    assert_eq!(recorded.0.oid, subject.oid);
    assert_eq!(recorded.1, "user-1");
}

#[rstest]
#[case::policy_unavailable(FailureCode::PolicyUnavailable, 503)]
#[case::identity_denied(FailureCode::IdentityDenied, 403)]
#[tokio::test]
async fn enrollment_failures_keep_their_api_status(#[case] code: FailureCode, #[case] status: u16) {
    let store = FakeEnroller {
        calls: AtomicUsize::new(0),
        recorded: Mutex::new(None),
        failure: Some(AgentIdentityFailure {
            code,
            message: String::from("the identity store refused the enrollment"),
        }),
    };
    let subject = microsoft_subject();
    let error = enroll_microsoft_subject(Some(&subject), Some("user-1"), &store)
        .await
        .expect_err("the enrollment failure");
    assert_eq!(error.status, status);
    assert_eq!(error.message, "the identity store refused the enrollment");
}
