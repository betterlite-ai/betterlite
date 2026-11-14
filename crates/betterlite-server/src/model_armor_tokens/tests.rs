#![cfg(test)]

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_config::sections::vertex_credential_sources::credential_source::{
    ConfiguredCredentials, CredentialSource,
};
use betterlite_guardrails::model_armor::transport::tokens::ArmorTokens;
use betterlite_vertex::credential_sources::error::CredentialSourceError;
use betterlite_vertex::credential_sources::sdk::{CredentialClock, SystemClock};
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::{CredentialExecutor, LoadedCredential};
use betterlite_vertex::token_cache::record::CredentialSnapshot;
use betterlite_vertex::token_cache::state::TokenState;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;

use super::VertexArmorTokens;
use super::error::ArmorTokenError;

const CREDENTIALS_TEXT: &str = "credentials-text";
const AUTHORIZED_USER: &str = r#"{"type":"authorized_user","client_id":"client","client_secret":"CLIENTSECRET","refresh_token":"REFRESHTOKEN","quota_project_id":"quota-project"}"#;
const MALFORMED_AUTHORIZED_USER: &str = r#"{"type":"authorized_user","client_id":"client","client_secret":"CLIENTSECRET","refresh_token":"REFRESHTOKEN","token_uri":7}"#;

type LoadResult = Result<LoadedCredential, CredentialCacheError>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum SeenCredentials {
    AuthorizedUser,
    ServiceAccount,
    OtherSource,
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LookupRequest {
    credentials: Option<SeenCredentials>,
    project: Option<String>,
}

struct FakeExecutor {
    loads: Mutex<VecDeque<LoadResult>>,
    requests: Mutex<Vec<LookupRequest>>,
    load_calls: AtomicUsize,
    refresh_calls: AtomicUsize,
}

impl FakeExecutor {
    fn scripted(loads: Vec<LoadResult>) -> Self {
        Self {
            loads: Mutex::new(VecDeque::from(loads)),
            requests: Mutex::new(Vec::new()),
            load_calls: AtomicUsize::new(0),
            refresh_calls: AtomicUsize::new(0),
        }
    }
}

fn seen(credentials: Option<&ConfiguredCredentials>) -> Option<SeenCredentials> {
    match credentials? {
        ConfiguredCredentials::Source(CredentialSource::AuthorizedUser(_)) => {
            Some(SeenCredentials::AuthorizedUser)
        }
        ConfiguredCredentials::Source(CredentialSource::ServiceAccount(_)) => {
            Some(SeenCredentials::ServiceAccount)
        }
        ConfiguredCredentials::Source(_) => Some(SeenCredentials::OtherSource),
        ConfiguredCredentials::Text(text) => {
            Some(SeenCredentials::Text(text.expose_secret().to_owned()))
        }
    }
}

impl CredentialExecutor for FakeExecutor {
    fn load<'borrow>(
        &'borrow self,
        credentials: Option<&'borrow ConfiguredCredentials>,
        project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(LookupRequest {
            credentials: seen(credentials),
            project: project.map(str::to_owned),
        });
        let next = self.loads.lock().unwrap().pop_front();
        Box::pin(
            async move { next.unwrap_or_else(|| Err(CredentialCacheError::MissingCredentials)) },
        )
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(CredentialCacheError::MissingToken) })
    }
}

struct Harness {
    executor: Arc<FakeExecutor>,
    clock: Arc<dyn CredentialClock>,
    tasks: TaskTracker,
}

impl Harness {
    fn new(loads: Vec<LoadResult>) -> Self {
        Self {
            executor: Arc::new(FakeExecutor::scripted(loads)),
            clock: Arc::new(SystemClock),
            tasks: TaskTracker::new(),
        }
    }

    fn tokens(
        &self,
        credentials: Option<&str>,
        project: Option<&str>,
    ) -> Result<VertexArmorTokens, ArmorTokenError> {
        let executor: Arc<dyn CredentialExecutor> = Arc::clone(&self.executor);
        VertexArmorTokens::new(
            credentials.map(SecretString::from),
            project.map(str::to_owned),
            Arc::clone(&self.clock),
            executor,
            self.tasks.clone(),
        )
    }

    fn requests(&self) -> Vec<LookupRequest> {
        self.executor.requests.lock().unwrap().clone()
    }
}

fn snapshot(token: &str, project: Option<&str>) -> CredentialSnapshot {
    CredentialSnapshot {
        token: Some(SecretString::from(token)),
        state: Some(TokenState::Fresh),
        expires_at: None,
        expired: None,
        valid: None,
        quota_project_id: None,
        project_id: project.map(str::to_owned),
    }
}

fn loaded(token: &str, project: Option<&str>) -> LoadedCredential {
    LoadedCredential {
        credential: Some(Arc::new(snapshot(token, project))),
        project: project.map(str::to_owned),
    }
}

#[tokio::test]
async fn tokens_reuses_a_fresh_token_without_a_second_exchange() {
    let harness = Harness::new(vec![Ok(loaded("token-1", Some("credential-project")))]);
    let tokens = harness.tokens(Some(CREDENTIALS_TEXT), None).unwrap();
    let first = tokens.access_token().await.unwrap();
    let second = tokens.access_token().await.unwrap();
    assert_eq!(first.0.expose_secret(), "token-1");
    assert_eq!(first.1.as_deref(), Some("credential-project"));
    assert_eq!(second.0.expose_secret(), "token-1");
    assert_eq!(second.1.as_deref(), Some("credential-project"));
    assert_eq!(harness.executor.load_calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.executor.refresh_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn tokens_keeps_one_credential_cache_per_guardrail() {
    let harness = Harness::new(vec![
        Ok(loaded("token-a", Some("project-a"))),
        Ok(loaded("token-b", Some("project-b"))),
    ]);
    let first = harness.tokens(Some(CREDENTIALS_TEXT), None).unwrap();
    let second = harness.tokens(Some(CREDENTIALS_TEXT), None).unwrap();
    let first_access = first.access_token().await.unwrap();
    let second_access = second.access_token().await.unwrap();
    assert_eq!(first_access.0.expose_secret(), "token-a");
    assert_eq!(second_access.0.expose_secret(), "token-b");
    assert_eq!(harness.executor.load_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn tokens_fills_an_absent_project_from_the_credentials() {
    let harness = Harness::new(vec![Ok(loaded("token", Some("credential-project")))]);
    let tokens = harness.tokens(Some(CREDENTIALS_TEXT), None).unwrap();
    let (_token, project) = tokens.access_token().await.unwrap();
    assert_eq!(project.as_deref(), Some("credential-project"));
}

#[tokio::test]
async fn tokens_passes_the_configured_project_to_the_lookup() {
    let harness = Harness::new(vec![Ok(loaded("token", Some("configured-project")))]);
    let tokens = harness
        .tokens(Some(CREDENTIALS_TEXT), Some("configured-project"))
        .unwrap();
    let (_token, project) = tokens.access_token().await.unwrap();
    assert_eq!(project.as_deref(), Some("configured-project"));
    assert_eq!(
        harness.requests(),
        vec![LookupRequest {
            credentials: Some(SeenCredentials::Text(CREDENTIALS_TEXT.to_owned())),
            project: Some("configured-project".to_owned()),
        }]
    );
}

#[tokio::test]
async fn tokens_decodes_json_credentials_before_the_lookup() {
    let harness = Harness::new(vec![Ok(loaded("token", Some("quota-project")))]);
    let tokens = harness.tokens(Some(AUTHORIZED_USER), None).unwrap();
    let (_token, project) = tokens.access_token().await.unwrap();
    assert_eq!(project.as_deref(), Some("quota-project"));
    assert_eq!(
        harness.requests(),
        vec![LookupRequest {
            credentials: Some(SeenCredentials::AuthorizedUser),
            project: None,
        }]
    );
}

#[tokio::test]
async fn tokens_hands_a_credentials_path_to_the_decoder() {
    let harness = Harness::new(vec![Ok(loaded("token", Some("file-project")))]);
    let tokens = harness.tokens(Some("/run/vertex.json"), None).unwrap();
    let (_token, project) = tokens.access_token().await.unwrap();
    assert_eq!(project.as_deref(), Some("file-project"));
    assert_eq!(
        harness.requests(),
        vec![LookupRequest {
            credentials: Some(SeenCredentials::Text("/run/vertex.json".to_owned())),
            project: None,
        }]
    );
}

#[tokio::test]
async fn tokens_uses_default_credentials_when_none_are_configured() {
    let harness = Harness::new(vec![Ok(loaded("adc-token", Some("adc-project")))]);
    let tokens = harness.tokens(None, None).unwrap();
    let (token, project) = tokens.access_token().await.unwrap();
    assert_eq!(token.expose_secret(), "adc-token");
    assert_eq!(project.as_deref(), Some("adc-project"));
    assert_eq!(
        harness.requests(),
        vec![LookupRequest {
            credentials: None,
            project: None,
        }]
    );
}

#[tokio::test]
async fn tokens_refuses_unreadable_credentials_with_the_lite_llm_text() {
    let harness = Harness::new(Vec::new());
    let error = harness
        .tokens(Some(MALFORMED_AUTHORIZED_USER), None)
        .unwrap_err();
    assert!(matches!(&error, ArmorTokenError::Credentials(_)));
    let rendered = error.to_string();
    assert!(
        rendered.contains("Unable to load vertex credentials from environment"),
        "{rendered}"
    );
    assert!(rendered.contains("Parse error:"), "{rendered}");
    assert!(!rendered.contains("CLIENTSECRET"), "{rendered}");
    assert!(!rendered.contains("REFRESHTOKEN"), "{rendered}");
}

#[tokio::test]
async fn tokens_debug_hides_the_credentials_and_the_token() {
    let harness = Harness::new(vec![Ok(loaded("SECRET-ACCESS-TOKEN", Some("project")))]);
    let tokens = harness
        .tokens(Some(AUTHORIZED_USER), Some("project"))
        .unwrap();
    let before = format!("{tokens:?}");
    assert!(before.contains("VertexArmorTokens"), "{before}");
    assert!(before.contains("credentials: true"), "{before}");
    assert!(before.contains("project"), "{before}");
    assert!(!before.contains("CLIENTSECRET"), "{before}");
    assert!(!before.contains("REFRESHTOKEN"), "{before}");
    let (token, _project) = tokens.access_token().await.unwrap();
    assert_eq!(token.expose_secret(), "SECRET-ACCESS-TOKEN");
    let after = format!("{tokens:?}");
    assert!(!after.contains("SECRET-ACCESS-TOKEN"), "{after}");
    assert!(!after.contains("CLIENTSECRET"), "{after}");
    assert!(!after.contains("REFRESHTOKEN"), "{after}");
}

#[tokio::test]
async fn tokens_lookup_errors_hide_the_configured_credentials() {
    let harness = Harness::new(vec![Err(CredentialCacheError::Load(
        CredentialSourceError::MissingProject,
    ))]);
    let tokens = harness.tokens(Some(AUTHORIZED_USER), None).unwrap();
    let error = tokens.access_token().await.unwrap_err();
    assert!(matches!(&error, ArmorTokenError::Lookup(_)));
    let debug = format!("{error:?}");
    assert!(!debug.contains("CLIENTSECRET"), "{debug}");
    assert!(!debug.contains("REFRESHTOKEN"), "{debug}");
    let rendered = error.to_string();
    assert!(!rendered.contains("CLIENTSECRET"), "{rendered}");
}
