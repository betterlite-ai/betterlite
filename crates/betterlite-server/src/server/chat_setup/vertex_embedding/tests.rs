#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::SourceError;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_tokenizer::engine::limits::BlockingCounter as _;
use betterlite_vertex::aws_federation::AwsCredentialsResolver as _;
use betterlite_vertex::credential_sources::error::CredentialSourceError;
use betterlite_vertex::credential_sources::sdk::{
    CredentialClock, GoogleAuth, HttpPolicy, TokenSource,
};
use betterlite_vertex::gemini_embedding_records::usage::TextTokenCounter as _;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::CredentialExecutor as _;
use betterlite_vertex::token_cache::record::CredentialSnapshot;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use tokio_util::task::TaskTracker;

use super::counter::{InlineCounter, RuntimeTextCounter};
use super::executor::{UnavailableAws, VertexCredentialExecutor};
use super::{VertexEmbeddingSetupError, token_counter, vertex_embedding_calls};
use crate::server::chat_setup::chat_executor;

const EPOCH_SECONDS: u64 = 1_000;
// the fixed clock plus the executor's assumed 600 second token lifetime
const REFRESHED_EXPIRY_SECONDS: u64 = 1_600;

struct FixedClock;

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(EPOCH_SECONDS))
            .unwrap()
    }
}

struct FixedSource(&'static str);

impl TokenSource for FixedSource {
    fn access_token(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<SecretString, CredentialSourceError>> + Send + '_>>
    {
        Box::pin(async move { Ok(SecretString::from(self.0.to_owned())) })
    }
}

fn digest(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

fn build_executor(handles: BTreeMap<String, Arc<dyn TokenSource>>) -> VertexCredentialExecutor {
    VertexCredentialExecutor {
        environment: AdcEnvironment::default(),
        google: GoogleAuth {
            client: client(),
            policy: HttpPolicy::default(),
        },
        clock: Arc::new(FixedClock),
        handles: Mutex::new(handles),
    }
}

fn snapshot(token: Option<&str>, project: Option<&str>) -> CredentialSnapshot {
    CredentialSnapshot {
        token: token.map(|text| SecretString::from(text.to_owned())),
        state: None,
        expires_at: None,
        expired: None,
        valid: None,
        quota_project_id: None,
        project_id: project.map(str::to_owned),
    }
}

fn load_config(model_list: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(model_list)
    .unwrap()
}

const OPENAI_ONLY: &str = "model_list:\n  - model_name: chat\n    litellm_params:\n      model: openai/gpt-4o\n      api_key: key\n";
const VERTEX: &str = "model_list:\n  - model_name: embed\n    litellm_params:\n      model: vertex_ai/text-embedding-004\n      vertex_project: pinned-project\n";

#[test]
fn a_config_without_a_vertex_deployment_builds_no_call() {
    let config = load_config(OPENAI_ONLY);
    let chat = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).unwrap();
    let calls = vertex_embedding_calls(&config, &chat, &TaskTracker::new()).unwrap();
    assert!(calls.is_none());
}

#[test]
fn a_vertex_deployment_builds_the_call() {
    let config = load_config(VERTEX);
    let chat = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).unwrap();
    let calls = vertex_embedding_calls(&config, &chat, &TaskTracker::new()).unwrap();
    assert!(calls.is_some());
}

#[test]
fn a_pin_that_is_not_text_or_an_object_names_the_deployment() {
    let yaml = "model_list:\n  - model_name: embed\n    litellm_params:\n      model: vertex_ai/text-embedding-004\n      vertex_credentials: 5\n";
    let config = load_config(yaml);
    let chat = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).unwrap();
    let Err(VertexEmbeddingSetupError::Pinned { model_name, .. }) =
        vertex_embedding_calls(&config, &chat, &TaskTracker::new())
    else {
        panic!("a numeric credential pin must stop startup");
    };
    assert_eq!(model_name, "embed");
}

#[tokio::test]
async fn an_aws_federated_credential_has_no_provider() {
    let result = UnavailableAws.credentials(&AwsSettings::default()).await;
    assert!(matches!(
        result,
        Err(CredentialError::Source(SourceError::MissingProvider))
    ));
}

#[tokio::test]
async fn refresh_moves_the_handle_to_the_new_token() {
    let source: Arc<dyn TokenSource> = Arc::new(FixedSource("new-token"));
    let executor = build_executor(BTreeMap::from([(digest("old-token"), source)]));
    let stale = snapshot(Some("old-token"), Some("project"));
    let refreshed = executor.refresh(&stale).await.unwrap();
    assert_eq!(refreshed.token.unwrap().expose_secret(), "new-token");
    assert_eq!(refreshed.project_id.as_deref(), Some("project"));
    assert_eq!(
        refreshed.expires_at,
        SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(REFRESHED_EXPIRY_SECONDS))
    );
    let handles = executor.handles.lock().unwrap();
    assert!(handles.contains_key(&digest("new-token")));
    assert!(!handles.contains_key(&digest("old-token")));
    // the map holds digests, so no key is a plain token
    assert!(!handles.contains_key("new-token"));
    assert!(handles.keys().all(|key| key.len() == 64));
    drop(handles);
}

#[tokio::test]
async fn refresh_of_an_untracked_token_fails() {
    let executor = build_executor(BTreeMap::new());
    let result = executor
        .refresh(&snapshot(Some("unknown"), Some("project")))
        .await;
    assert!(matches!(
        result,
        Err(CredentialCacheError::MissingCredentials)
    ));
}

#[tokio::test]
async fn refresh_without_a_token_fails() {
    let executor = build_executor(BTreeMap::new());
    let result = executor.refresh(&snapshot(None, Some("project"))).await;
    assert!(matches!(result, Err(CredentialCacheError::MissingToken)));
}

#[tokio::test]
async fn load_reports_an_unreadable_credential() {
    let executor = build_executor(BTreeMap::new());
    let credentials = ConfiguredCredentials::from_text("garbage".to_owned()).unwrap();
    let result = executor.load(Some(&credentials), Some("project")).await;
    assert!(matches!(result, Err(CredentialCacheError::Load(_))));
    assert!(executor.handles.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_inline_counter_runs_each_job_where_it_is_called() {
    let counted = InlineCounter.count(Box::new(|| Ok(7))).await.unwrap();
    assert_eq!(counted, 7);
    let converted = InlineCounter
        .convert(Box::new(|| Ok(vec![3, 4])))
        .await
        .unwrap();
    assert_eq!(converted, vec![3, 4]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_text_counter_counts_with_the_bundled_vocabulary() {
    let config = load_config(VERTEX);
    let catalog = BundledCatalog::default().get();
    let counter = RuntimeTextCounter {
        counter: token_counter(&config, &Arc::new(client()), &catalog).unwrap(),
    };
    assert_eq!(
        counter
            .count_text("gemini-embedding-001", "hello world")
            .unwrap(),
        2
    );
}
