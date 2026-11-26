#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_config::tls::Verification;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::metadata::identity::Mode;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::realtime_backend_transport::dial::RealtimeSsl;
use crate::api::realtime_connect::token::{
    VertexAccessTokenSource, VertexTokenError, VertexTokenFuture,
};

use super::azure::AzureAdTokens;
use super::metadata::{ModelInfoFailure, RealtimeModelInfo, RealtimeModelMetadata};
use super::vertex::{
    VertexProbeConfig, VertexProbeEndpoint, VertexProbeParameters, VertexProbeTarget,
    VertexTokenSource,
};
use super::{
    ProbeConnection, ProbeDialError, RealtimeProbeDependencies, RealtimeProbeDialer,
    RealtimeProbeError, RealtimeProbeRequest, RealtimeProbeSettings, realtime_health_check,
};

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

fn empty_credentials() -> CredentialSnapshot {
    CredentialSnapshot::default()
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
        credential_timeout_seconds: 20.0,
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
        match self.failure.lock().unwrap().clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

struct StaticMetadata {
    entries: BTreeMap<String, Result<Option<RealtimeModelInfo>, ModelInfoFailure>>,
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
        self.entries.insert(
            String::from(model),
            Err(ModelInfoFailure {
                message: String::from("get_model_info raised"),
            }),
        );
        self
    }
}

impl RealtimeModelMetadata for StaticMetadata {
    fn model_info(
        &self,
        model: &str,
        _custom_llm_provider: &str,
    ) -> Result<Option<RealtimeModelInfo>, ModelInfoFailure> {
        self.entries.get(model).cloned().unwrap_or(Ok(None))
    }
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
        self.calls.lock().unwrap().push(params.clone());
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
    tokens: Arc<ScriptedTokens>,
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
        let source: Arc<VertexTokenSource> = self.tokens.clone();
        source
    }

    fn live(&self, target: &VertexProbeTarget, _api_base: Option<&str>) -> VertexProbeEndpoint {
        self.targets.lock().unwrap().push(target.clone());
        self.endpoint.clone()
    }
}

fn empty_vertex() -> FakeVertex {
    FakeVertex::new(EMPTY_CATALOG)
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

#[tokio::test]
async fn realtime_probe_stored_precedence() {
    let stored = credentials(&[(
        "c",
        &[
            ("api_key", text("stored-key")),
            ("api_base", text("https://stored.test")),
            ("api_version", text("stored-version")),
        ],
    )]);
    let metadata = StaticMetadata::new();
    let azure = ScriptedAzure::default();
    let vertex = empty_vertex();
    let dialer = RecordingDialer::default();
    let base = settings();
    let probe = Probe {
        stored: &stored,
        metadata: &metadata,
        azure: &azure,
        vertex: &vertex,
        dialer: &dialer,
        verification: &Verification::Enabled,
    };

    // named nonempty arguments beat the merged map
    let mut named = probe_request(
        "azure",
        MODEL,
        params(&[("litellm_credential_name", text("c"))]),
    );
    named.api_key = Some(SecretString::from("named-key"));
    named.api_base = Some(String::from("https://named.test"));
    named.api_version = Some(String::from("named-version"));
    named.realtime_protocol = Some(String::from("beta"));
    probe.run(&named, &base).await.unwrap();
    let connection = dialer.last();
    assert_eq!(
        connection.url,
        "wss://named.test/openai/realtime?api-version=named-version&deployment=realtime-a"
    );
    assert_eq!(
        connection.headers,
        Headers::from([(String::from("api-key"), String::from("named-key"))])
    );
    assert!(azure.calls().is_empty(), "a truthy key skips Azure AD");

    // the stored values sit underneath the explicit parameters
    let mut merged = probe_request(
        "azure",
        MODEL,
        params(&[("litellm_credential_name", text("c"))]),
    );
    merged.realtime_protocol = Some(String::from("beta"));
    probe.run(&merged, &base).await.unwrap();
    let connection = dialer.last();
    assert_eq!(
        connection.url,
        "wss://stored.test/openai/realtime?api-version=stored-version&deployment=realtime-a"
    );
    assert_eq!(
        connection.headers,
        Headers::from([(String::from("api-key"), String::from("stored-key"))])
    );
    assert!(azure.calls().is_empty());
}

#[tokio::test]
async fn realtime_probe_explicit_none_erases_a_stored_value() {
    let stored = credentials(&[(
        "c",
        &[
            ("api_key", text("stored-key")),
            ("api_base", text("https://stored.test")),
            ("api_version", text("stored-version")),
        ],
    )]);
    let metadata = StaticMetadata::new();
    let azure = ScriptedAzure::default();
    let vertex = empty_vertex();
    let dialer = RecordingDialer::default();
    let probe = Probe {
        stored: &stored,
        metadata: &metadata,
        azure: &azure,
        vertex: &vertex,
        dialer: &dialer,
        verification: &Verification::Enabled,
    };
    let mut request = probe_request(
        "azure",
        MODEL,
        params(&[
            ("litellm_credential_name", text("c")),
            ("api_key", Json::Null),
            ("api_base", Json::Null),
            ("api_version", Json::Null),
        ]),
    );
    request.realtime_protocol = Some(String::from("beta"));
    probe.run(&request, &settings()).await.unwrap();
    let connection = dialer.last();
    assert_eq!(
        connection.url, "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a",
        "an explicit None erases the stored value and the defaults apply"
    );
    let calls = azure.calls();
    assert_eq!(calls.len(), 1, "an absent key resolves an Azure AD token");
    assert_eq!(calls[0].get("api_key"), Some(&Json::Null));
}

#[tokio::test]
async fn realtime_probe_credential_name_lookup_rules() {
    let stored = credentials(&[
        ("", &[("api_base", text("https://empty-name.test"))]),
        ("7", &[("api_base", text("https://numbered.test"))]),
    ]);
    let metadata = StaticMetadata::new();
    let azure = ScriptedAzure::default();
    let vertex = empty_vertex();
    let dialer = RecordingDialer::default();
    let probe = Probe {
        stored: &stored,
        metadata: &metadata,
        azure: &azure,
        vertex: &vertex,
        dialer: &dialer,
        verification: &Verification::Enabled,
    };
    let empty_name = probe_request(
        "openai",
        MODEL,
        params(&[("litellm_credential_name", text(""))]),
    );
    probe.run(&empty_name, &settings()).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "wss://empty-name.test/v1/realtime?model=realtime-a",
        "an empty string still reaches the credential accessor"
    );
    let non_string = probe_request(
        "openai",
        MODEL,
        params(&[(
            "litellm_credential_name",
            Json::Number(serde_json::Number::from(7_i64)),
        )]),
    );
    probe.run(&non_string, &settings()).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "wss://api.openai.com/v1/realtime?model=realtime-a",
        "a non-string credential name performs no lookup"
    );
}

#[tokio::test]
async fn realtime_probe_openai_auth_headers() {
    let stored = empty_credentials();
    let metadata = StaticMetadata::new();
    let azure = ScriptedAzure::default();
    let vertex = empty_vertex();
    let dialer = RecordingDialer::default();
    let probe = Probe {
        stored: &stored,
        metadata: &metadata,
        azure: &azure,
        vertex: &vertex,
        dialer: &dialer,
        verification: &Verification::Enabled,
    };
    let absent = probe_request("openai", MODEL, JsonObject::new());
    probe.run(&absent, &settings()).await.unwrap();
    assert!(
        dialer.last().headers.is_empty(),
        "an absent key sends no auth header"
    );
    let empty = probe_request("openai", MODEL, params(&[("api_key", text(""))]));
    probe.run(&empty, &settings()).await.unwrap();
    assert_eq!(
        dialer.last().headers,
        Headers::from([(String::from("Authorization"), String::from("Bearer "))]),
        "an empty key still sends a Bearer header"
    );
}

#[tokio::test]
async fn realtime_probe_azure_resolves_ad_from_the_merged_parameters() {
    let stored = credentials(&[(
        "c",
        &[
            ("litellm_credential_name", text("c")),
            ("azure_ad_token", text("stored-ad-token")),
        ],
    )]);
    let metadata = StaticMetadata::new();
    let azure = ScriptedAzure::with_token("resolved-ad-token");
    let vertex = empty_vertex();
    let dialer = RecordingDialer::default();
    let probe = Probe {
        stored: &stored,
        metadata: &metadata,
        azure: &azure,
        vertex: &vertex,
        dialer: &dialer,
        verification: &Verification::Enabled,
    };
    let empty_key = probe_request(
        "azure",
        MODEL,
        params(&[
            ("litellm_credential_name", text("c")),
            ("api_key", text("")),
        ]),
    );
    probe.run(&empty_key, &settings()).await.unwrap();
    let calls = azure.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].get("azure_ad_token"),
        Some(&text("stored-ad-token")),
        "the accessor reads the merged map"
    );
    assert_eq!(
        dialer.last().headers,
        Headers::from([(
            String::from("Authorization"),
            String::from("Bearer resolved-ad-token")
        )])
    );
}

#[tokio::test]
async fn realtime_probe_azure_protocol() {
    let stored = empty_credentials();
    let metadata = StaticMetadata::new()
        .with_transcription(MODEL)
        .with_endpoints("endpoint-model", &["/v1/realtime/transcription_sessions"])
        .with_endpoints("other-endpoint", &["/v1/chat/completions"])
        .failing("failing-model");
    let azure = ScriptedAzure::with_token("ad-token");
    let vertex = empty_vertex();
    let dialer = RecordingDialer::default();
    let base = settings();
    let probe = Probe {
        stored: &stored,
        metadata: &metadata,
        azure: &azure,
        vertex: &vertex,
        dialer: &dialer,
        verification: &Verification::Enabled,
    };

    let transcription = probe_request("azure", MODEL, JsonObject::new());
    probe.run(&transcription, &base).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/v1/realtime?intent=transcription",
        "a transcription model supplies only intent=transcription with the default base"
    );
    let endpoint_model = probe_request("azure", "endpoint-model", JsonObject::new());
    probe.run(&endpoint_model, &base).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/v1/realtime?intent=transcription"
    );
    let other = probe_request("azure", "other-endpoint", JsonObject::new());
    probe.run(&other, &base).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/v1/realtime?model=other-endpoint"
    );
    let failing = probe_request("azure", "failing-model", JsonObject::new());
    probe.run(&failing, &base).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/v1/realtime?model=failing-model",
        "a failing metadata lookup is not a transcription model"
    );

    // named, then model parameters, then the environment; MODEL is transcription-only, so intent rides along
    let mut named = probe_request("azure", MODEL, params(&[("realtime_protocol", text("GA"))]));
    named.realtime_protocol = Some(String::from("beta"));
    let mut ga_environment = settings();
    ga_environment.azure_realtime_protocol = Some(String::from("GA"));
    probe.run(&named, &ga_environment).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription",
        "the named protocol wins over the model parameters"
    );

    let mut beta_environment = settings();
    beta_environment.azure_realtime_protocol = Some(String::from("beta"));
    let model_ga = probe_request("azure", MODEL, params(&[("realtime_protocol", text("GA"))]));
    probe.run(&model_ga, &beta_environment).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/v1/realtime?intent=transcription",
        "the model protocol wins over the environment"
    );

    let mut non_string = probe_request("azure", MODEL, JsonObject::new());
    non_string.model_params.insert(
        String::from("realtime_protocol"),
        Json::Number(serde_json::Number::from(1_i64)),
    );
    probe.run(&non_string, &beta_environment).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/v1/realtime?intent=transcription",
        "a truthy non-string model protocol suppresses the environment and falls back to GA"
    );

    let plain = probe_request("azure", MODEL, JsonObject::new());
    probe.run(&plain, &beta_environment).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription",
        "the environment supplies beta when nothing else is configured"
    );
    probe.run(&plain, &settings()).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/v1/realtime?intent=transcription",
        "no configured string gives GA"
    );
    let empty_version = probe_request("azure", MODEL, params(&[("api_version", text(""))]));
    probe.run(&empty_version, &beta_environment).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription",
        "an empty configured version takes the default"
    );

    // a falsy configured value falls through to the next truthy one
    let mut empty_named = probe_request("azure", MODEL, JsonObject::new());
    empty_named.realtime_protocol = Some(String::new());
    probe.run(&empty_named, &beta_environment).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription",
        "an empty named protocol is absent, so the environment applies"
    );
    let empty_list = probe_request(
        "azure",
        MODEL,
        params(&[("realtime_protocol", Json::List(Vec::new()))]),
    );
    probe.run(&empty_list, &beta_environment).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription",
        "an empty list is falsy"
    );
    let truthy_list = probe_request(
        "azure",
        MODEL,
        params(&[("realtime_protocol", Json::List(vec![text("GA")]))]),
    );
    probe.run(&truthy_list, &beta_environment).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/v1/realtime?intent=transcription",
        "a non-empty list is truthy and is not a string"
    );
    let empty_object = probe_request(
        "azure",
        MODEL,
        params(&[("realtime_protocol", Json::Object(JsonObject::new()))]),
    );
    probe.run(&empty_object, &beta_environment).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription",
        "an empty object is falsy"
    );
    let truthy_object = probe_request(
        "azure",
        MODEL,
        params(&[(
            "realtime_protocol",
            Json::Object(JsonObject::from([(String::from("value"), text("GA"))])),
        )]),
    );
    probe.run(&truthy_object, &beta_environment).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "/openai/v1/realtime?intent=transcription",
        "a non-empty object is truthy and is not a string"
    );
}

#[tokio::test]
async fn realtime_probe_openai_and_xai_defaults() {
    let stored = empty_credentials();
    let metadata = StaticMetadata::new().with_transcription(MODEL);
    let azure = ScriptedAzure::default();
    let vertex = empty_vertex();
    let dialer = RecordingDialer::default();
    let probe = Probe {
        stored: &stored,
        metadata: &metadata,
        azure: &azure,
        vertex: &vertex,
        dialer: &dialer,
        verification: &Verification::Enabled,
    };
    let mut openai = probe_request("openai", MODEL, JsonObject::new());
    openai.api_key = Some(SecretString::from("openai-key"));
    probe.run(&openai, &settings()).await.unwrap();
    let connection = dialer.last();
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
    probe.run(&xai, &settings()).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "wss://api.x.ai/v1/realtime?model=realtime-a"
    );
    let empty_base = probe_request("openai", MODEL, params(&[("api_base", text(""))]));
    probe.run(&empty_base, &settings()).await.unwrap();
    assert_eq!(
        dialer.last().url,
        "wss://api.openai.com/v1/realtime?model=realtime-a",
        "a configured empty base takes the default"
    );
}

#[tokio::test]
async fn realtime_probe_vertex_and_failures() {
    let chunk_catalog: &[u8] = br#"{"vertex_ai/chirp_2": {"litellm_provider": "vertex_ai", "mode": "audio_transcription", "supported_endpoints": ["/v1/audio/transcriptions", "/v1/realtime"]}}"#;
    let stored = empty_credentials();
    let metadata = StaticMetadata::new();
    let azure = ScriptedAzure::default();
    let dialer = RecordingDialer::default();
    let chunk_vertex = FakeVertex::new(chunk_catalog);
    let rejection = Probe {
        stored: &stored,
        metadata: &metadata,
        azure: &azure,
        vertex: &chunk_vertex,
        dialer: &dialer,
        verification: &Verification::Enabled,
    };
    let chirp = probe_request("vertex_ai", "chirp_2", JsonObject::new());
    let error = rejection
        .run(&chirp, &settings())
        .await
        .expect_err("a Speech-to-Text model is rejected");
    assert_eq!(
        error.to_string(),
        "Realtime health checks are not supported for Speech-to-Text streaming model chirp_2; health check it with mode audio_transcription"
    );
    assert_eq!(
        dialer.count(),
        0,
        "the rejection happens before any connect"
    );

    let vertex = empty_vertex();
    let probe = Probe {
        stored: &stored,
        metadata: &metadata,
        azure: &azure,
        vertex: &vertex,
        dialer: &dialer,
        verification: &Verification::Enabled,
    };
    let mut live = probe_request("vertex_ai", MODEL, JsonObject::new());
    live.api_key = Some(SecretString::from("caller-key"));
    live.api_base = Some(String::from("https://vertex-base.test"));
    probe.run(&live, &settings()).await.unwrap();
    let targets = vertex.targets();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].model, MODEL);
    assert_eq!(targets[0].project, "resolved-project");
    assert_eq!(targets[0].region, "us-central1");
    assert_eq!(targets[0].access_token.expose_secret(), "oauth-token");
    assert_eq!(vertex.configured_regions(), vec![None]);
    assert_eq!(
        dialer.last().headers,
        Headers::from([(
            String::from("Authorization"),
            String::from("Bearer oauth-token")
        )]),
        "a caller API key never replaces the OAuth Bearer header"
    );

    let mut timed_out = settings();
    timed_out.credential_timeout_seconds = 0.0;
    let error = probe
        .run(&live, &timed_out)
        .await
        .expect_err("a zero bound expires without polling");
    assert_eq!(
        error.to_string(),
        "Vertex AI realtime: timed out fetching Google OAuth access token after 0.0s; check network egress from the proxy to the OAuth token endpoint (oauth2.googleapis.com)"
    );

    let unsupported = probe_request("bedrock", MODEL, JsonObject::new());
    let error = probe
        .run(&unsupported, &settings())
        .await
        .expect_err("an unsupported provider is a ValueError");
    assert_eq!(error.to_string(), "Unsupported model: realtime-a");

    dialer.fail(ProbeDialError::new(
        "InvalidStatus",
        String::from("server rejected WebSocket connection: HTTP 401"),
    ));
    let refused = probe
        .run(&live, &settings())
        .await
        .expect_err("a refused handshake propagates instead of returning false");
    match refused {
        RealtimeProbeError::Dial(error) => assert_eq!(error.status, None),
        other => panic!("a refused handshake is a dial error, got {other:?}"),
    }
    dialer.fail(ProbeDialError::new(
        "InvalidStatus",
        String::from("HTTP 503"),
    ));
    probe
        .run(&live, &settings())
        .await
        .expect_err("every handshake failure propagates");
    assert_eq!(
        dialer.count(),
        3,
        "the probe never retries a handshake: one live connect and two refused ones"
    );

    let mut broken = probe_request("openai", MODEL, JsonObject::new());
    broken.api_base = Some(String::from("::"));
    let error = probe
        .run(&broken, &settings())
        .await
        .expect_err("a URL build failure propagates");
    assert!(matches!(error, RealtimeProbeError::OpenAi(_)));
    assert_eq!(dialer.count(), 3, "a failed build never reaches the dialer");
}

#[tokio::test]
async fn realtime_probe_connection_settings() {
    let stored = empty_credentials();
    let metadata = StaticMetadata::new();
    let azure = ScriptedAzure::default();
    let vertex = empty_vertex();
    let dialer = RecordingDialer::default();
    let request = probe_request("openai", MODEL, JsonObject::new());
    let mut explicit = settings();
    explicit.max_message_size_bytes = 1024;
    let enabled = Probe {
        stored: &stored,
        metadata: &metadata,
        azure: &azure,
        vertex: &vertex,
        dialer: &dialer,
        verification: &Verification::Enabled,
    };
    enabled.run(&request, &explicit).await.unwrap();
    let connection = dialer.last();
    assert_eq!(connection.ssl, RealtimeSsl::Shared(Verification::Enabled));
    assert_eq!(connection.max_size, 1024);
    enabled.run(&request, &settings()).await.unwrap();
    assert_eq!(dialer.last().max_size, 16 * 1024 * 1024);

    let disabled = Probe {
        stored: &stored,
        metadata: &metadata,
        azure: &azure,
        vertex: &vertex,
        dialer: &dialer,
        verification: &Verification::Disabled,
    };
    disabled.run(&request, &settings()).await.unwrap();
    assert_eq!(dialer.last().ssl, RealtimeSsl::Insecure);
    let mut plaintext = probe_request("openai", MODEL, JsonObject::new());
    plaintext.api_base = Some(String::from("http://provider.test"));
    disabled.run(&plaintext, &settings()).await.unwrap();
    assert_eq!(
        dialer.last().ssl,
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
