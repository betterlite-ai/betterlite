#![cfg(test)]

use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use super::config::{InteractionCall, VertexInteractionsConfig};
use super::error::VertexInteractionsError;
use super::mint::{MintAccessToken, MintFuture, ProjectOnlyMint};
use super::target::VertexInteractionsTarget;
use super::{API_VERSION, DEFAULT_LOCATION, PROVIDER};
use crate::credential_sources::getters::VertexParams;
use crate::credential_sources::sdk::CredentialClock;
use crate::studio_interactions::urls::Lifecycle;
use crate::token_cache::CredentialCache;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::executor::{CredentialExecutor, LoadedCredential};
use crate::token_cache::probe::AccessToken;
use crate::token_cache::record::CredentialSnapshot;
use crate::token_cache::state::TokenState;

const GLOBAL_COLLECTION: &str =
    "https://aiplatform.googleapis.com/v1beta1/projects/project-1/locations/global/interactions";

#[derive(Clone, Copy)]
struct Minted {
    token: &'static str,
    project: &'static str,
}

struct RecordingMint {
    minted: Vec<Minted>,
    calls: Mutex<Vec<(Option<String>, Option<String>)>>,
}

impl RecordingMint {
    fn new(minted: Vec<Minted>) -> Self {
        Self {
            minted,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn recording() -> Self {
        Self::new(Vec::new())
    }

    fn calls(&self) -> Vec<(Option<String>, Option<String>)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(credentials, project)| (credentials.clone(), project.clone()))
            .collect()
    }
}

impl MintAccessToken for RecordingMint {
    fn mint<'mint>(
        &'mint self,
        credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        let mut recorded = self.calls.lock().unwrap();
        let index = recorded.len();
        recorded.push((
            credentials
                .and_then(ConfiguredCredentials::as_text)
                .map(str::to_owned),
            project.map(str::to_owned),
        ));
        let next = self.minted.get(index).copied();
        drop(recorded);
        Box::pin(async move {
            Ok(AccessToken {
                token: SecretString::from(next.map_or("token", |minted| minted.token)),
                project: next.map_or("project-1", |minted| minted.project).to_owned(),
            })
        })
    }
}

struct FailingMint;

impl MintAccessToken for FailingMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        _project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        Box::pin(ready(Err(CredentialCacheError::MissingCredentials)))
    }
}

struct FixedClock;

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

struct SnapshotExecutor {
    snapshot: Arc<CredentialSnapshot>,
}

impl CredentialExecutor for SnapshotExecutor {
    fn load<'borrow>(
        &'borrow self,
        _credentials: Option<&'borrow ConfiguredCredentials>,
        _project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        let snapshot = Arc::clone(&self.snapshot);
        Box::pin(async move {
            Ok(LoadedCredential {
                credential: Some(snapshot),
                project: None,
            })
        })
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(ready(Err(CredentialCacheError::MissingToken)))
    }
}

fn params<'params>(
    project: Option<&'params str>,
    location: Option<&'params str>,
    alias: Option<&'params str>,
) -> VertexParams<'params> {
    VertexParams {
        project,
        ai_project: None,
        location,
        ai_location: alias,
        credentials: None,
        ai_credentials: None,
    }
}

fn config<M>(mint: Arc<M>) -> VertexInteractionsConfig
where
    M: MintAccessToken + 'static,
{
    VertexInteractionsConfig { mint }
}

fn recording() -> VertexInteractionsConfig {
    config(Arc::new(RecordingMint::recording()))
}

fn call<'call>(
    api_base: Option<&'call str>,
    params: VertexParams<'call>,
    settings: &'call VertexSettings,
) -> InteractionCall<'call> {
    InteractionCall {
        api_base,
        params,
        settings,
    }
}

#[tokio::test]
async fn target_and_headers_mint_order() {
    let mint = Arc::new(RecordingMint::new(vec![
        Minted {
            token: "first-token",
            project: "first-project",
        },
        Minted {
            token: "second-token",
            project: "second-project",
        },
    ]));
    let settings = VertexSettings::default();
    let given = call(
        None,
        params(Some("requested-project"), None, None),
        &settings,
    );
    let target = config(Arc::clone(&mint)).target(&given).await.unwrap();
    assert_eq!(target.project_id, "first-project");
    assert_eq!(target.base_url, "https://aiplatform.googleapis.com");
    assert_eq!(target.location, DEFAULT_LOCATION);
    let headers = config(Arc::clone(&mint))
        .headers(&given, &IndexMap::new())
        .await
        .unwrap();
    assert_eq!(
        headers.get("Authorization").unwrap().expose_secret(),
        "Bearer second-token"
    );
    assert_eq!(
        mint.calls(),
        vec![
            (None, Some("requested-project".to_owned())),
            (None, Some("requested-project".to_owned())),
        ]
    );
}

#[tokio::test]
async fn lyria_project_only_callback() {
    let api = config(Arc::new(ProjectOnlyMint {
        project: "configured-project".to_owned(),
    }));
    let settings = VertexSettings::default();
    let given = params(Some("param-project"), Some("global"), None);
    assert_eq!(
        api.get_complete_url(&call(None, given, &settings), false)
            .await
            .unwrap(),
        "https://aiplatform.googleapis.com/v1beta1/projects/param-project/locations/global/interactions"
    );
    assert_eq!(
        api.get_complete_url(
            &call(None, params(None, Some("global"), None), &settings),
            false
        )
        .await
        .unwrap(),
        "https://aiplatform.googleapis.com/v1beta1/projects/configured-project/locations/global/interactions"
    );
    let headers = api
        .headers(&call(None, given, &settings), &IndexMap::new())
        .await
        .unwrap();
    assert_eq!(
        headers.get("Authorization").unwrap().expose_secret(),
        "Bearer "
    );
}

#[tokio::test]
async fn credential_cache_backs_the_default_mint() {
    let snapshot = Arc::new(CredentialSnapshot {
        token: Some(SecretString::from("cached-token")),
        state: Some(TokenState::Fresh),
        expires_at: None,
        expired: None,
        valid: Some(true),
        quota_project_id: None,
        project_id: None,
    });
    let cache = CredentialCache::new(
        Arc::new(FixedClock),
        Arc::new(SnapshotExecutor { snapshot }),
    );
    let minted = cache.mint(None, Some("project-1")).await.unwrap();
    assert_eq!(minted.project, "project-1");
    assert_eq!(minted.token.expose_secret(), "cached-token");
}

#[tokio::test]
async fn global_default_location() {
    let settings = VertexSettings::default();
    assert_eq!(
        recording()
            .get_complete_url(
                &call(None, params(Some("project-1"), None, None), &settings),
                false
            )
            .await
            .unwrap(),
        GLOBAL_COLLECTION
    );
}

#[rstest]
#[case::region("us-central1", "https://us-central1-aiplatform.googleapis.com")]
#[case::geography("us", "https://aiplatform.us.rep.googleapis.com")]
#[tokio::test]
async fn explicit_location_selects_the_host(#[case] location: &str, #[case] host: &str) {
    let settings = VertexSettings::default();
    let expected = format!("{host}/v1beta1/projects/project-1/locations/{location}/interactions");
    assert_eq!(
        recording()
            .get_complete_url(
                &call(
                    None,
                    params(Some("project-1"), Some(location), None),
                    &settings
                ),
                false
            )
            .await
            .unwrap(),
        expected
    );
}

#[rstest]
#[case::geography("eu", "https://aiplatform.eu.rep.googleapis.com")]
#[case::region("us-central1", "https://us-central1-aiplatform.googleapis.com")]
#[tokio::test]
async fn location_alias_selects_the_host(#[case] alias: &str, #[case] host: &str) {
    let settings = VertexSettings::default();
    let expected = format!("{host}/v1beta1/projects/project-1/locations/{alias}/interactions");
    assert_eq!(
        recording()
            .get_complete_url(
                &call(
                    None,
                    params(Some("project-1"), None, Some(alias)),
                    &settings
                ),
                false
            )
            .await
            .unwrap(),
        expected
    );
}

#[rstest]
#[case::custom(Some("https://proxy.example/v1"), "https://proxy.example/v1")]
#[case::empty(Some(""), "https://aiplatform.googleapis.com")]
#[tokio::test]
async fn custom_base_is_used_verbatim(#[case] api_base: Option<&str>, #[case] base: &str) {
    let settings = VertexSettings::default();
    let expected = format!("{base}/v1beta1/projects/project-1/locations/global/interactions");
    assert_eq!(
        recording()
            .get_complete_url(
                &call(api_base, params(Some("project-1"), None, None), &settings),
                false,
            )
            .await
            .unwrap(),
        expected
    );
}

#[tokio::test]
async fn invalid_location_fails_even_with_a_custom_base() {
    let settings = VertexSettings::default();
    let error = recording()
        .get_complete_url(
            &call(
                Some("https://proxy.example"),
                params(Some("project-1"), Some("attacker.example/"), None),
                &settings,
            ),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, VertexInteractionsError::Location(_)));
}

#[tokio::test]
async fn missing_project_fails() {
    let mint = Arc::new(RecordingMint::new(vec![Minted {
        token: "token",
        project: "",
    }]));
    let settings = VertexSettings::default();
    let error = config(mint)
        .get_complete_url(&call(None, params(None, None, None), &settings), false)
        .await
        .unwrap_err();
    assert!(matches!(error, VertexInteractionsError::MissingProject));
}

#[tokio::test]
async fn mint_failure_is_carried() {
    let settings = VertexSettings::default();
    let error = config(Arc::new(FailingMint))
        .target(&call(
            None,
            params(Some("project-1"), None, None),
            &settings,
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, VertexInteractionsError::Mint(_)));
}

#[tokio::test]
async fn settings_project_and_credentials_reach_the_mint() {
    let mint = Arc::new(RecordingMint::recording());
    let settings = VertexSettings {
        project: Some("settings-project".to_owned()),
        location: None,
        credentials: Some(ConfiguredCredentials::from_text("service-key".to_owned()).unwrap()),
        adc: AdcEnvironment::default(),
    };
    config(Arc::clone(&mint))
        .target(&call(None, params(None, None, None), &settings))
        .await
        .unwrap();
    assert_eq!(
        mint.calls(),
        vec![(
            Some("service-key".to_owned()),
            Some("settings-project".to_owned())
        )]
    );
}

#[tokio::test]
async fn caller_headers_override_json_and_bearer() {
    let settings = VertexSettings::default();
    let mut caller = IndexMap::new();
    caller.insert("Authorization".to_owned(), "Bearer caller".to_owned());
    caller.insert("Content-Type".to_owned(), "text/plain".to_owned());
    caller.insert("X-Extra".to_owned(), "1".to_owned());
    let headers = recording()
        .headers(
            &call(None, params(Some("project-1"), None, None), &settings),
            &caller,
        )
        .await
        .unwrap();
    assert_eq!(
        headers.get("Authorization").unwrap().expose_secret(),
        "Bearer caller"
    );
    assert_eq!(
        headers.get("Content-Type").unwrap().expose_secret(),
        "text/plain"
    );
    assert_eq!(headers.get("X-Extra").unwrap().expose_secret(), "1");
}

#[tokio::test]
async fn vertex_headers_add_no_revision_or_api_key() {
    let settings = VertexSettings::default();
    let headers = recording()
        .headers(
            &call(None, params(Some("project-1"), None, None), &settings),
            &IndexMap::new(),
        )
        .await
        .unwrap();
    let names: Vec<&str> = headers.keys().map(String::as_str).collect();
    assert_eq!(names, vec!["Content-Type", "Authorization"]);
    assert_eq!(
        headers.get("Content-Type").unwrap().expose_secret(),
        "application/json"
    );
    assert_eq!(
        headers.get("Authorization").unwrap().expose_secret(),
        "Bearer token"
    );
}

#[rstest]
#[case::get(Lifecycle::Get, "")]
#[case::delete(Lifecycle::Delete, "")]
#[case::cancel(Lifecycle::Cancel, ":cancel")]
#[tokio::test]
async fn encoded_interaction_id_is_one_segment(#[case] action: Lifecycle, #[case] suffix: &str) {
    let settings = VertexSettings::default();
    let request = recording()
        .lifecycle_request(
            &call(None, params(Some("project-1"), None, None), &settings),
            action,
            "a/b c?d#e%",
        )
        .await
        .unwrap();
    assert_eq!(
        request.url,
        format!("{GLOBAL_COLLECTION}/a%2Fb%20c%3Fd%23e%25{suffix}")
    );
    assert_eq!(serde_json::to_string(&request.body).unwrap(), "{}");
}

#[rstest]
#[case::empty("")]
#[case::dot(".")]
#[case::dot_dot("..")]
#[tokio::test]
async fn invalid_interaction_ids_are_refused(#[case] interaction_id: &str) {
    let settings = VertexSettings::default();
    let error = recording()
        .lifecycle_request(
            &call(None, params(Some("project-1"), None, None), &settings),
            Lifecycle::Get,
            interaction_id,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, VertexInteractionsError::InteractionId(_)));
}

#[tokio::test]
async fn streaming_collection_appends_alt_sse() {
    let settings = VertexSettings::default();
    assert_eq!(
        recording()
            .get_complete_url(
                &call(None, params(Some("project-1"), None, None), &settings),
                true
            )
            .await
            .unwrap(),
        format!("{GLOBAL_COLLECTION}?alt=sse")
    );
}

#[test]
fn provider_identity_and_version() {
    assert_eq!(PROVIDER, ProviderId::VertexAi);
    assert_eq!(serde_json::to_value(PROVIDER).unwrap(), json!("vertex_ai"));
    assert_eq!(API_VERSION, "v1beta1");
    assert_eq!(DEFAULT_LOCATION, "global");
}

#[test]
fn target_collection_url_keeps_project_spelling() {
    let target = VertexInteractionsTarget {
        base_url: "https://aiplatform.googleapis.com".to_owned(),
        project_id: "my project/with:colon".to_owned(),
        location: "global".to_owned(),
    };
    assert_eq!(
        target.collection_url(),
        "https://aiplatform.googleapis.com/v1beta1/projects/my project/with:colon/locations/global/interactions"
    );
}
