#![cfg(test)]

use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use betterlite_config::sections::studio_identity::Settings;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::{
    ConfiguredCredentials, decode_vertex_credentials,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::{Content, Message};
use http::header::HeaderMap;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;

use crate::credential_sources::getters::VertexParams;
use crate::credential_sources::sdk::CredentialClock;
use crate::google_token_counter::counter::{
    CountRuntime, CountTokensCall, StudioTokenCounter, TokenCounter as _, VertexTokenCounter,
};
use crate::google_token_counter::deployment::DeploymentParams;
use crate::google_token_counter::error::{CountTokensError, ModelInfoError};
use crate::google_token_counter::model_info::{GeminiModelInfo, VertexModelInfo};
use crate::google_token_counter::reply::{CountReply, TokenCountResponse};
use crate::partner_count::execute::PartnerCountTransport;
use crate::token_cache::CredentialCache;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::executor::{CredentialExecutor, LoadedCredential};
use crate::token_cache::probe::AccessToken;
use crate::token_cache::record::CredentialSnapshot;
use crate::token_cache::state::TokenState;
use crate::token_refresh::CredentialRefresh;
use crate::vertex_interactions::mint::{MintAccessToken, MintFuture};

const DEADLINE: Duration = Duration::from_secs(5);

const VERTEX_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.0-flash:countTokens";
const STUDIO_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:countTokens";
const CONVERTED: &str = r#"{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}"#;
const PARTNER_BODY: &str =
    r#"{"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"hello"}]}"#;
const ALIAS_PARTNER_URL: &str = "https://europe-west1-aiplatform.googleapis.com/v1/projects/alias-project/locations/europe-west1/publishers/anthropic/models/count-tokens:rawPredict";
const ASIA_PARTNER_URL: &str = "https://asia-southeast1-aiplatform.googleapis.com/v1/projects/primary-project/locations/asia-southeast1/publishers/anthropic/models/count-tokens:rawPredict";
const EAST5_PARTNER_URL: &str = "https://us-east5-aiplatform.googleapis.com/v1/projects/primary-project/locations/us-east5/publishers/anthropic/models/count-tokens:rawPredict";
const PRIMARY_PARTNER_URL: &str = "https://asia-southeast1-aiplatform.googleapis.com/v1/projects/primary-project/locations/asia-southeast1/publishers/anthropic/models/count-tokens:rawPredict";

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn message(text: &str) -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text(text.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn params<'call>(
    project: Option<&'call str>,
    location: Option<&'call str>,
    credentials: Option<&'call ConfiguredCredentials>,
) -> VertexParams<'call> {
    VertexParams {
        project,
        ai_project: None,
        location,
        ai_location: None,
        credentials,
        ai_credentials: None,
    }
}

fn deployment<'call>(
    model: &'call str,
    vertex: VertexParams<'call>,
    count_tokens_location: Option<&'call str>,
) -> DeploymentParams<'call> {
    DeploymentParams {
        model: Some(model),
        api_key: None,
        gemini_api_key: None,
        api_base: None,
        timeout: None,
        vertex,
        vertex_count_tokens_location: count_tokens_location,
    }
}

fn call<'call>(
    model_to_use: &'call str,
    messages: &'call [Message],
    contents: Option<&'call Json>,
    options: DeploymentParams<'call>,
) -> CountTokensCall<'call> {
    CountTokensCall {
        model_to_use,
        messages,
        contents,
        deployment: options,
        request_model: model_to_use,
        tools: None,
        system: None,
    }
}

fn vertex_counter() -> VertexTokenCounter {
    VertexTokenCounter {
        settings: VertexSettings::default(),
    }
}

struct Sent {
    url: String,
    body: Vec<u8>,
}

impl Sent {
    fn body_json(&self) -> Json {
        serde_json::from_slice(&self.body).unwrap()
    }
}

struct Script {
    reply: Vec<u8>,
    sent: Mutex<Vec<Sent>>,
}

impl Script {
    fn replying(body: &[u8]) -> Self {
        Self {
            reply: body.to_vec(),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, request: OutboundRequest) -> UpstreamResponse {
        let body = match request.body {
            RequestBody::Json(bytes) => bytes,
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Vec::new()
            }
        };
        self.sent.lock().unwrap().push(Sent {
            url: request.url.to_string(),
            body,
        });
        UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: self.reply.clone(),
        }
    }

    fn sent(&self) -> Vec<Sent> {
        let mut sent = self.sent.lock().unwrap();
        std::mem::take(&mut *sent)
    }
}

impl CountTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Ok(self.record(request)))
    }
}

impl PartnerCountTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Ok(self.record(request)))
    }
}

struct NoMedia;

impl RemoteMediaFetcher for NoMedia {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Rejected {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(Err(ImageFetchError::Rejected {
            url: urls.join(","),
        }))
    }
}

static NO_MEDIA: NoMedia = NoMedia;

#[derive(Default)]
struct RecordingMint {
    seen: Mutex<Vec<(Option<String>, Option<String>)>>,
}

impl RecordingMint {
    fn seen(&self) -> Vec<(Option<String>, Option<String>)> {
        self.seen.lock().unwrap().clone()
    }
}

impl MintAccessToken for RecordingMint {
    fn mint<'mint>(
        &'mint self,
        credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        self.seen.lock().unwrap().push((
            credentials
                .and_then(ConfiguredCredentials::as_text)
                .map(str::to_owned),
            project.map(str::to_owned),
        ));
        Box::pin(ready(Ok(AccessToken {
            token: SecretString::from("minted"),
            project: project.unwrap_or("minted-project").to_owned(),
        })))
    }
}

struct FixedClock;

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

struct StaticExecutor;

impl CredentialExecutor for StaticExecutor {
    fn load<'borrow>(
        &'borrow self,
        _credentials: Option<&'borrow ConfiguredCredentials>,
        _project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        let snapshot = CredentialSnapshot {
            token: Some(SecretString::from("vertex-token")),
            state: Some(TokenState::Fresh),
            expires_at: None,
            expired: None,
            valid: None,
            quota_project_id: None,
            project_id: Some("proj".to_owned()),
        };
        Box::pin(ready(Ok(LoadedCredential {
            credential: Some(Arc::new(snapshot)),
            project: Some("proj".to_owned()),
        })))
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

fn refresh() -> CredentialRefresh {
    CredentialRefresh::new(
        Arc::new(CredentialCache::new(
            Arc::new(FixedClock),
            Arc::new(StaticExecutor),
        )),
        TaskTracker::new(),
    )
}

fn runtime<'run>(
    script: &'run Script,
    mint: &'run RecordingMint,
    refresh: &'run CredentialRefresh,
    catalog: &'run ModelCatalog,
) -> CountRuntime<'run, Script, RecordingMint, NoMedia> {
    CountRuntime {
        transport: script,
        refresh,
        catalog,
        mint,
        fetcher: &NO_MEDIA,
    }
}

async fn vertex_count(
    script: &Script,
    mint: &RecordingMint,
    refresh: &CredentialRefresh,
    catalog: &ModelCatalog,
    request: &CountTokensCall<'_>,
) -> Result<Option<TokenCountResponse>, CountTokensError> {
    let counter = vertex_counter();
    let live = runtime(script, mint, refresh, catalog);
    counter.count_tokens(&live, request, DEADLINE).await
}

#[tokio::test]
async fn selection_empty_contents_wins() {
    let script = Script::replying(br#"{"totalTokens":5,"tokenizer_used":"google"}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj"), Some("us-central1"), None),
        None,
    );
    let contents = json("[]");
    let messages = [message("hello")];
    let request = call("gemini-2.0-flash", &messages, Some(&contents), options);
    let response = vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap()
        .expect("a reply with totalTokens");
    assert_eq!(response.total_tokens, 5);
    assert_eq!(response.tokenizer_type, "google");
    assert_eq!(response.request_model, "gemini-2.0-flash");
    assert!(!response.error);
    assert!(response.error_message.is_none());
    assert!(response.status_code.is_none());
    assert!(matches!(
        &response.original_response,
        Some(CountReply::Google(reply)) if reply.total_tokens == Some(5)
    ));
    let sent = script.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].url, VERTEX_URL);
    assert_eq!(sent[0].body_json(), json(r#"{"contents":[]}"#));
}

#[tokio::test]
async fn selection_vertex_message_fallback() {
    let script = Script::replying(br#"{"totalTokens":3}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj"), Some("us-central1"), None),
        None,
    );
    let messages = [message("hello")];
    let request = CountTokensCall {
        model_to_use: "gemini-2.0-flash",
        messages: &messages,
        contents: None,
        deployment: options,
        request_model: "request-model",
        tools: None,
        system: None,
    };
    let response = vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap()
        .expect("a converted history counts");
    assert_eq!(response.total_tokens, 3);
    assert_eq!(response.request_model, "request-model");
    assert_eq!(response.model_used, "gemini-2.0-flash");
    assert_eq!(response.tokenizer_type, "");
    assert_eq!(script.sent()[0].body_json(), json(CONVERTED));
}

#[tokio::test]
async fn selection_vertex_null_contents() {
    let script = Script::replying(br#"{"totalTokens":3}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj"), Some("us-central1"), None),
        None,
    );
    let messages = [message("hello")];
    let missing = Json::Null;
    let request = call("gemini-2.0-flash", &messages, Some(&missing), options);
    vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap();
    assert_eq!(script.sent()[0].body_json(), json(CONVERTED));
}

#[tokio::test]
async fn selection_tools_and_system_are_ignored() {
    let script = Script::replying(br#"{"totalTokens":2}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj"), Some("us-central1"), None),
        None,
    );
    let contents = json("[]");
    let tools = json(r#"[{"functionDeclarations":[]}]"#);
    let system = json(r#""be brief""#);
    let request = CountTokensCall {
        model_to_use: "gemini-2.0-flash",
        messages: &[],
        contents: Some(&contents),
        deployment: options,
        request_model: "gemini-2.0-flash",
        tools: Some(&tools),
        system: Some(&system),
    };
    vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap();
    assert_eq!(script.sent()[0].body_json(), json(r#"{"contents":[]}"#));
}

#[tokio::test]
async fn selection_studio_none_contents() {
    let script = Script::replying(br#"{"totalTokens":9,"tokenizer_used":"google"}"#);
    let options = deployment("gemini/gemini-2.0-flash", params(None, None, None), None);
    let messages = [message("hello")];
    let request = call("gemini-2.0-flash", &messages, None, options);
    let counter = StudioTokenCounter {
        settings: Settings::default(),
    };
    let response = counter
        .count_tokens(&script, &request, DEADLINE)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 9);
    assert_eq!(response.tokenizer_type, "google");
    let sent = script.sent();
    assert_eq!(sent[0].url, STUDIO_URL);
    assert_eq!(sent[0].body_json(), json(r#"{"contents":null}"#));
}

#[tokio::test]
async fn selection_vertex_missing_total() {
    let script = Script::replying(br#"{}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj"), Some("us-central1"), None),
        None,
    );
    let contents = json("[]");
    let messages = [];
    let request = call("gemini-2.0-flash", &messages, Some(&contents), options);
    let outcome = vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap();
    assert!(outcome.is_none());
    let null_total = Script::replying(br#"{"totalTokens":null}"#);
    let outcome = vertex_count(&null_total, &mint, &refresh, &catalog, &request)
        .await
        .unwrap();
    assert!(outcome.is_none());
    let overflowing = Script::replying(br#"{"totalTokens":18446744073709551615}"#);
    let error = vertex_count(&overflowing, &mint, &refresh, &catalog, &request)
        .await
        .expect_err("a count past i64::MAX is refused");
    assert!(matches!(error, CountTokensError::Overflow));
}

#[tokio::test]
async fn selection_studio_missing_total() {
    let script = Script::replying(br#"{}"#);
    let options = deployment("gemini/gemini-2.0-flash", params(None, None, None), None);
    let messages = [];
    let request = call("gemini-2.0-flash", &messages, None, options);
    let counter = StudioTokenCounter {
        settings: Settings::default(),
    };
    let response = counter
        .count_tokens(&script, &request, DEADLINE)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 0);
    assert_eq!(response.tokenizer_type, "");
    assert!(matches!(
        response.original_response,
        Some(CountReply::Google(_))
    ));
    let null_total = Script::replying(br#"{"totalTokens":null}"#);
    let response = counter
        .count_tokens(&null_total, &request, DEADLINE)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 0);
}

#[tokio::test]
async fn selection_partner_aliases() {
    let script =
        Script::replying(br#"{"input_tokens":11,"tokenizer_used":"vertex_ai_partner_models"}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let empty = ConfiguredCredentials::Text(SecretString::from(""));
    let alias = ConfiguredCredentials::Text(SecretString::from("alias-credentials"));
    let options = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        VertexParams {
            project: Some(""),
            ai_project: Some("alias-project"),
            location: Some(""),
            ai_location: Some("europe-west1"),
            credentials: Some(&empty),
            ai_credentials: Some(&alias),
        },
        None,
    );
    let messages = [message("hello")];
    let request = call("claude-sonnet-4-6@default", &messages, None, options);
    let response = vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap()
        .expect("a partner count reply");
    assert_eq!(response.total_tokens, 11);
    assert_eq!(response.tokenizer_type, "vertex_ai_partner_models");
    assert!(matches!(
        response.original_response,
        Some(CountReply::Partner(_))
    ));
    assert_eq!(mint.seen()[0].0.as_deref(), Some("alias-credentials"));
    assert_eq!(mint.seen()[0].1.as_deref(), Some("alias-project"));
    let sent = script.sent();
    assert_eq!(sent[0].url, ALIAS_PARTNER_URL);
    assert_eq!(sent[0].body_json(), json(PARTNER_BODY));
}

#[tokio::test]
async fn selection_partner_empty_messages() {
    let script = Script::replying(br#"{"input_tokens":0}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let options = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        params(Some("proj"), Some("us-east5"), None),
        None,
    );
    let request = call("claude-sonnet-4-6@default", &[], None, options);
    let response = vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap()
        .expect("a partner count reply");
    assert_eq!(response.total_tokens, 0);
    assert_eq!(
        script.sent()[0].body_json(),
        json(r#"{"model":"claude-sonnet-4-6","messages":[]}"#)
    );
}

#[tokio::test]
async fn selection_partner_primary_spelling_wins() {
    let script = Script::replying(br#"{"input_tokens":1}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let primary = ConfiguredCredentials::Text(SecretString::from("primary-credentials"));
    let alias = ConfiguredCredentials::Text(SecretString::from("alias-credentials"));
    let options = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        VertexParams {
            project: Some("primary-project"),
            ai_project: Some("alias-project"),
            location: Some("asia-southeast1"),
            ai_location: Some("europe-west1"),
            credentials: Some(&primary),
            ai_credentials: Some(&alias),
        },
        None,
    );
    let messages = [message("hello")];
    let request = call("claude-sonnet-4-6@default", &messages, None, options);
    vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap();
    assert_eq!(script.sent()[0].url, PRIMARY_PARTNER_URL);
    assert_eq!(mint.seen()[0].0.as_deref(), Some("primary-credentials"));
    assert_eq!(mint.seen()[0].1.as_deref(), Some("primary-project"));
}

#[tokio::test]
async fn selection_partner_count_tokens_location_wins() {
    let script = Script::replying(br#"{"input_tokens":1}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let messages = [message("hello")];
    let pinned = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        params(Some("primary-project"), Some("us-east5"), None),
        Some("asia-southeast1"),
    );
    let pinned_call = call("claude-sonnet-4-6@default", &messages, None, pinned);
    vertex_count(&script, &mint, &refresh, &catalog, &pinned_call)
        .await
        .unwrap();
    let plain = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        params(Some("primary-project"), Some("us-east5"), None),
        None,
    );
    let plain_call = call("claude-sonnet-4-6@default", &messages, None, plain);
    vertex_count(&script, &mint, &refresh, &catalog, &plain_call)
        .await
        .unwrap();
    let sent = script.sent();
    assert_eq!(sent[0].url, ASIA_PARTNER_URL);
    assert_eq!(sent[1].url, EAST5_PARTNER_URL);
}

#[tokio::test]
async fn deployment_isolation() {
    let script = Script::replying(br#"{"totalTokens":4}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let credentials = ConfiguredCredentials::Text(SecretString::from("credentials"));
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj"), Some("us-central1"), Some(&credentials)),
        Some("asia-southeast1"),
    );
    let messages = [];
    let first = call("gemini-2.0-flash", &messages, None, options);
    let second = call("gemini-2.5-pro", &messages, None, options);
    vertex_count(&script, &mint, &refresh, &catalog, &first)
        .await
        .unwrap();
    vertex_count(&script, &mint, &refresh, &catalog, &second)
        .await
        .unwrap();
    assert_eq!(options.model, Some("vertex_ai/gemini-2.0-flash"));
    assert_eq!(options.vertex.project, Some("proj"));
    assert_eq!(
        options.vertex_count_tokens_location,
        Some("asia-southeast1")
    );
    assert_eq!(
        options
            .vertex
            .credentials
            .and_then(ConfiguredCredentials::as_text),
        Some("credentials")
    );
    let sent = script.sent();
    assert!(sent[0].url.contains("/models/gemini-2.0-flash:countTokens"));
    assert!(sent[1].url.contains("/models/gemini-2.5-pro:countTokens"));
    assert!(sent[1].url.contains("/projects/proj/"));
    assert!(sent[1].url.contains("/locations/us-central1/"));
}

#[test]
fn selection_provider_predicates() {
    let vertex = vertex_counter();
    let studio = StudioTokenCounter {
        settings: Settings::default(),
    };
    assert!(vertex.should_use_token_counting_api(Some("vertex_ai")));
    assert!(!vertex.should_use_token_counting_api(Some("vertex_ai_beta")));
    assert!(!vertex.should_use_token_counting_api(Some("gemini")));
    assert!(!vertex.should_use_token_counting_api(None));
    assert!(studio.should_use_token_counting_api(Some("gemini")));
    assert!(!studio.should_use_token_counting_api(Some("gemini/")));
    assert!(!studio.should_use_token_counting_api(Some("vertex_ai")));
    assert!(!studio.should_use_token_counting_api(None));
}

#[test]
fn selection_location_alias_precedence() {
    let pinned = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        params(Some("proj"), Some("us-east5"), None),
        Some("asia-southeast1"),
    );
    assert_eq!(pinned.explicit_location(), Some("asia-southeast1"));
    let plain = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        params(Some("proj"), Some("us-east5"), None),
        Some(""),
    );
    assert_eq!(plain.explicit_location(), Some("us-east5"));
    let aliased = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        VertexParams {
            project: Some("proj"),
            ai_project: None,
            location: Some(""),
            ai_location: Some("europe-west1"),
            credentials: None,
            ai_credentials: None,
        },
        None,
    );
    assert_eq!(aliased.explicit_location(), Some("europe-west1"));
    let missing = deployment("m", params(None, None, None), None);
    assert_eq!(missing.explicit_location(), None);
}

#[test]
fn selection_source_credentials_are_named() {
    let document = concat!(
        r#"{"type":"service_account","project_id":"proj","#,
        r#""private_key":"-----BEGIN PRIVATE KEY-----\nkey\n-----END PRIVATE KEY-----\n","#,
        r#""client_email":"service@example.com"}"#
    );
    let source =
        ConfiguredCredentials::Source(decode_vertex_credentials(document.as_bytes()).unwrap());
    let text = ConfiguredCredentials::Text(SecretString::from("alias-credentials"));
    let options = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        VertexParams {
            project: Some("proj"),
            ai_project: None,
            location: Some("us-east5"),
            ai_location: None,
            credentials: Some(&source),
            ai_credentials: Some(&text),
        },
        None,
    );
    assert!(matches!(
        options.explicit_credentials(),
        Some(ConfiguredCredentials::Source(_))
    ));
}

#[test]
fn selection_model_info_stubs_stay_unsupported() {
    let vertex = VertexModelInfo {
        settings: VertexSettings::default(),
    };
    assert!(matches!(
        vertex.validate_environment(),
        Err(ModelInfoError::NotSupported)
    ));
    assert!(matches!(
        vertex.get_models(),
        Err(ModelInfoError::NotSupported)
    ));
    assert!(matches!(
        VertexModelInfo::get_api_key(),
        Err(ModelInfoError::NotSupported)
    ));
    assert!(matches!(
        VertexModelInfo::get_api_base(),
        Err(ModelInfoError::NotSupported)
    ));
    assert!(matches!(
        VertexModelInfo::get_base_model("gemini-2.0-flash"),
        Err(ModelInfoError::NotSupported)
    ));
    assert_eq!(
        ModelInfoError::NotSupported.to_string(),
        "Vertex AI models are not supported yet"
    );
    let studio = GeminiModelInfo {
        settings: Settings::default(),
    };
    assert_eq!(
        studio.get_token_counter().kind(),
        "GoogleAIStudioTokenCounter"
    );
    assert_eq!(vertex.get_token_counter().kind(), "VertexAITokenCounter");
}
