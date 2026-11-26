#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_config::tls::Verification;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::lookup::info::ModelInfoError;
use betterlite_models::metadata::identity::Mode;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

use crate::api::realtime_backend_transport::dial::RealtimeSsl;
use crate::api::realtime_connect::token::{
    VertexAccessTokenSource, VertexTokenError, VertexTokenFuture,
};

use super::azure::AzureAdTokens;
use super::metadata::{RealtimeModelInfo, RealtimeModelMetadata};
use super::vertex::{
    VertexProbeConfig, VertexProbeEndpoint, VertexProbeParameters, VertexProbeTarget,
    VertexTokenSource,
};
use super::{
    ProbeConnection, ProbeDialError, RealtimeProbeDependencies, RealtimeProbeDialer,
    RealtimeProbeError, RealtimeProbeRequest, RealtimeProbeSettings, realtime_health_check,
};

mod azure;
mod stored;
mod vertex;

const MODEL: &str = "realtime-a";
const EMPTY_CATALOG: &[u8] = b"{}";

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn params(pairs: &[(&str, Json)]) -> JsonObject {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn credentials(pairs: &[(&str, &[(&str, Json)])]) -> CredentialSnapshot {
    CredentialSnapshot::new(
        pairs
            .iter()
            .map(|(name, values)| CredentialEntry {
                credential_name: (*name).to_owned(),
                credential_values: params(values),
            })
            .collect(),
    )
}

fn catalog(bytes: &[u8]) -> Arc<ModelCatalog> {
    Arc::new(ModelCatalog::parse(bytes, UnknownFields::Reject).expect("a catalog"))
}

fn probe_request(provider: &str, model: &str, model_params: JsonObject) -> RealtimeProbeRequest {
    RealtimeProbeRequest {
        model: String::from(model),
        custom_llm_provider: String::from(provider),
        api_key: None,
        api_base: None,
        api_version: None,
        realtime_protocol: None,
        model_params,
    }
}

fn settings() -> RealtimeProbeSettings {
    RealtimeProbeSettings {
        azure_realtime_protocol: None,
        credential_timeout_seconds: 20.0_f64,
        max_message_size_bytes: 16 * 1024 * 1024,
    }
}

#[derive(Default)]
struct RecordingDialer {
    connections: Mutex<Vec<ProbeConnection>>,
    failure: Mutex<Option<ProbeDialError>>,
}

impl RecordingDialer {
    fn fail(&self, error: ProbeDialError) {
        *self.failure.lock().unwrap() = Some(error);
    }

    // the newest handshake; a test that asserts a count reads count() instead
    fn last(&self) -> ProbeConnection {
        let connections = self.connections.lock().unwrap();
        connections
            .last()
            .expect("the probe opens a connection")
            .clone()
    }

    fn count(&self) -> usize {
        self.connections.lock().unwrap().len()
    }
}

#[async_trait]
impl RealtimeProbeDialer for RecordingDialer {
    async fn probe(&self, connection: &ProbeConnection) -> Result<(), ProbeDialError> {
        self.connections.lock().unwrap().push(connection.clone());
        let failure = self.failure.lock().unwrap().clone();
        failure.map_or(Ok(()), Err)
    }
}

struct StaticMetadata {
    entries: BTreeMap<String, Result<Option<RealtimeModelInfo>, ()>>,
}

impl StaticMetadata {
    fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    fn with_transcription(mut self, model: &str) -> Self {
        let info = RealtimeModelInfo {
            mode: Some(Mode::AudioTranscription),
            supported_endpoints: Vec::new(),
        };
        self.entries.insert(String::from(model), Ok(Some(info)));
        self
    }

    fn with_endpoints(mut self, model: &str, endpoints: &[&str]) -> Self {
        let info = RealtimeModelInfo {
            mode: Some(Mode::Realtime),
            supported_endpoints: endpoints.iter().map(|value| (*value).to_owned()).collect(),
        };
        self.entries.insert(String::from(model), Ok(Some(info)));
        self
    }

    fn failing(mut self, model: &str) -> Self {
        self.entries.insert(model.to_owned(), Err(()));
        self
    }
}

impl RealtimeModelMetadata for StaticMetadata {
    fn model_info(
        &self,
        model: &str,
        custom_llm_provider: &str,
    ) -> Result<Option<RealtimeModelInfo>, ModelInfoError> {
        self.entries
            .get(model)
            .cloned()
            .unwrap_or(Ok(None))
            .map_err(|()| ModelInfoError::Unmapped {
                model: model.to_owned(),
                provider: custom_llm_provider.to_owned(),
            })
    }
}

#[test]
fn realtime_probe_metadata_keeps_typed_error() {
    let metadata = catalog(
        br#"{"fallback_generalizations":{"rules":[{"name":"bad-mode","pattern":"broken","model_info":{"mode":42}}]}}"#,
    );
    let error = metadata.model_info("broken", "azure").unwrap_err();
    assert!(matches!(error, ModelInfoError::Generalization(_)));
}

#[derive(Default)]
struct ScriptedAzure {
    token: Option<SecretString>,
    calls: Mutex<Vec<JsonObject>>,
}

impl ScriptedAzure {
    fn with_token(token: &str) -> Self {
        Self {
            token: Some(SecretString::from(token.to_owned())),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<JsonObject> {
        self.calls.lock().unwrap().clone()
    }
}

impl AzureAdTokens for ScriptedAzure {
    fn token(&self, params: &JsonObject) -> Result<Option<SecretString>, RealtimeProbeError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(params.clone());
        Ok(self.token.clone())
    }
}

struct ScriptedTokens {
    token: SecretString,
    project: String,
}

impl VertexAccessTokenSource for ScriptedTokens {
    type Error = VertexTokenError;

    fn resolve<'source>(
        &'source self,
        _credentials: Option<&'source Json>,
        _project_id: Option<&'source str>,
    ) -> VertexTokenFuture<'source> {
        let token = self.token.clone();
        let project = self.project.clone();
        Box::pin(async move { Ok((token, project)) })
    }
}

struct FakeVertex {
    catalog: Arc<ModelCatalog>,
    region: String,
    parameters: VertexProbeParameters,
    tokens: Arc<VertexTokenSource>,
    endpoint: VertexProbeEndpoint,
    targets: Mutex<Vec<VertexProbeTarget>>,
    configured_regions: Mutex<Vec<Option<String>>>,
}

impl FakeVertex {
    fn new(catalog_bytes: &[u8]) -> Self {
        Self {
            catalog: catalog(catalog_bytes),
            region: String::from("us-central1"),
            parameters: VertexProbeParameters::default(),
            tokens: Arc::new(ScriptedTokens {
                token: SecretString::from("oauth-token"),
                project: String::from("resolved-project"),
            }),
            endpoint: VertexProbeEndpoint {
                url: String::from("wss://vertex.test/ws/live"),
                headers: Headers::from([(
                    String::from("Authorization"),
                    String::from("Bearer oauth-token"),
                )]),
            },
            targets: Mutex::new(Vec::new()),
            configured_regions: Mutex::new(Vec::new()),
        }
    }

    fn targets(&self) -> Vec<VertexProbeTarget> {
        self.targets.lock().unwrap().clone()
    }

    fn configured_regions(&self) -> Vec<Option<String>> {
        self.configured_regions.lock().unwrap().clone()
    }
}

impl VertexProbeConfig for FakeVertex {
    fn catalog(&self) -> Arc<ModelCatalog> {
        Arc::clone(&self.catalog)
    }

    fn region(&self, _model: &str, configured: Option<&str>) -> String {
        self.configured_regions
            .lock()
            .unwrap()
            .push(configured.map(str::to_owned));
        self.region.clone()
    }

    fn parameters(&self, _params: &JsonObject) -> VertexProbeParameters {
        self.parameters.clone()
    }

    fn tokens(&self) -> Arc<VertexTokenSource> {
        Arc::clone(&self.tokens)
    }

    fn live(&self, target: &VertexProbeTarget, _api_base: Option<&str>) -> VertexProbeEndpoint {
        self.targets.lock().unwrap().push(target.clone());
        self.endpoint.clone()
    }
}

struct Probe<'harness> {
    stored: &'harness CredentialSnapshot,
    metadata: &'harness StaticMetadata,
    azure: &'harness ScriptedAzure,
    vertex: &'harness FakeVertex,
    dialer: &'harness RecordingDialer,
    verification: &'harness Verification,
}

impl Probe<'_> {
    async fn run(
        &self,
        request: &RealtimeProbeRequest,
        settings: &RealtimeProbeSettings,
    ) -> Result<(), RealtimeProbeError> {
        let deps = RealtimeProbeDependencies {
            stored: self.stored,
            metadata: self.metadata,
            azure: self.azure,
            vertex: self.vertex,
            dialer: self.dialer,
            verification: self.verification,
        };
        realtime_health_check(request, settings, &deps).await
    }
}

struct Harness {
    stored: CredentialSnapshot,
    metadata: StaticMetadata,
    azure: ScriptedAzure,
    vertex: FakeVertex,
    dialer: RecordingDialer,
    verification: Verification,
}

impl Harness {
    fn new() -> Self {
        Self {
            stored: CredentialSnapshot::default(),
            metadata: StaticMetadata::new(),
            azure: ScriptedAzure::default(),
            vertex: FakeVertex::new(EMPTY_CATALOG),
            dialer: RecordingDialer::default(),
            verification: Verification::Enabled,
        }
    }

    fn probe(&self) -> Probe<'_> {
        Probe {
            stored: &self.stored,
            metadata: &self.metadata,
            azure: &self.azure,
            vertex: &self.vertex,
            dialer: &self.dialer,
            verification: &self.verification,
        }
    }

    async fn run(
        &self,
        request: &RealtimeProbeRequest,
        settings: &RealtimeProbeSettings,
    ) -> Result<(), RealtimeProbeError> {
        self.probe().run(request, settings).await
    }
}

#[tokio::test]
async fn realtime_probe_openai_and_xai_defaults() {
    let harness = Harness {
        metadata: StaticMetadata::new().with_transcription(MODEL),
        ..Harness::new()
    };
    let mut openai = probe_request("openai", MODEL, JsonObject::new());
    openai.api_key = Some(SecretString::from("openai-key"));
    harness.run(&openai, &settings()).await.unwrap();
    let connection = harness.dialer.last();
    assert_eq!(
        connection.url, "wss://api.openai.com/v1/realtime?model=realtime-a",
        "OpenAI always supplies query model, even for a transcription-named model"
    );
    assert_eq!(
        connection.headers,
        Headers::from([(
            String::from("Authorization"),
            String::from("Bearer openai-key")
        )])
    );
    let xai = probe_request("xai", MODEL, JsonObject::new());
    harness.run(&xai, &settings()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "wss://api.x.ai/v1/realtime?model=realtime-a"
    );
    let empty_base = probe_request("openai", MODEL, params(&[("api_base", text(""))]));
    harness.run(&empty_base, &settings()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "wss://api.openai.com/v1/realtime?model=realtime-a",
        "a configured empty base takes the default"
    );
}

#[tokio::test]
async fn realtime_probe_connection_settings() {
    let mut harness = Harness::new();
    let request = probe_request("openai", MODEL, JsonObject::new());
    let mut explicit = settings();
    explicit.max_message_size_bytes = 1024;
    harness.run(&request, &explicit).await.unwrap();
    let connection = harness.dialer.last();
    assert_eq!(connection.ssl, RealtimeSsl::Shared(Verification::Enabled));
    assert_eq!(connection.max_size, 1024);
    harness.run(&request, &settings()).await.unwrap();
    assert_eq!(harness.dialer.last().max_size, 16 * 1024 * 1024);
    harness.verification = Verification::Disabled;
    harness.run(&request, &settings()).await.unwrap();
    assert_eq!(harness.dialer.last().ssl, RealtimeSsl::Insecure);
    let mut plaintext = probe_request("openai", MODEL, JsonObject::new());
    plaintext.api_base = Some(String::from("http://provider.test"));
    harness.run(&plaintext, &settings()).await.unwrap();
    assert_eq!(
        harness.dialer.last().ssl,
        RealtimeSsl::Disabled,
        "a plaintext URL passes no TLS context"
    );
}

// the impl B2225's probe reads through when the metadata comes from the model catalog
#[test]
fn realtime_probe_catalog_metadata_reads_mode_and_endpoints() {
    let catalog = catalog(
        br#"{"vertex_ai/chirp_2": {"litellm_provider": "vertex_ai", "mode": "audio_transcription", "supported_endpoints": ["/v1/audio/transcriptions", "/v1/realtime"]}, "openai/gpt-realtime": {"litellm_provider": "openai", "mode": "realtime"}}"#,
    );
    let chirp = catalog
        .model_info("chirp_2", "vertex_ai")
        .unwrap()
        .expect("the catalog maps chirp_2");
    assert_eq!(chirp.mode, Some(Mode::AudioTranscription));
    assert_eq!(
        chirp.supported_endpoints,
        vec!["/v1/audio/transcriptions", "/v1/realtime"]
    );
    let live = catalog
        .model_info("gpt-realtime", "openai")
        .unwrap()
        .expect("the catalog maps gpt-realtime");
    assert_eq!(live.mode, Some(Mode::Realtime));
    assert!(live.supported_endpoints.is_empty());
    assert_eq!(
        catalog.model_info("missing-model", "vertex_ai").unwrap(),
        None
    );
}
