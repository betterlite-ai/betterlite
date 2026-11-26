use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::tls::Verification;
use betterlite_core::api::realtime_connect::token::{
    VertexAccessTokenSource, VertexTokenError, VertexTokenFuture,
};
use betterlite_core::api::realtime_connection_plan::azure::AzureAdTokens;
use betterlite_core::api::realtime_connection_plan::vertex::{
    VertexProbeConfig as _, VertexProbeTarget, VertexTokenSource,
};
use betterlite_core::api::realtime_connection_plan::{
    ProbeConnection, ProbeDialError, RealtimeProbeDialer, RealtimeProbeError, RealtimeProbeSettings,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::realtime_probe::{RealtimeProbe, VertexLiveProbe};
use super::super::seams::RealtimeProbeCall as _;
use super::support::{mode_parameters, text};
use crate::health_probe_modes::calls::RealtimeCall;

// an unsupported provider never reads Azure or Vertex, so these stand in for their real resolvers
struct UnusedAzure;

impl AzureAdTokens for UnusedAzure {
    fn token(&self, _params: &JsonObject) -> Result<Option<SecretString>, RealtimeProbeError> {
        Ok(None)
    }
}

struct UnusedDialer;

#[async_trait]
impl RealtimeProbeDialer for UnusedDialer {
    async fn probe(&self, _connection: &ProbeConnection) -> Result<(), ProbeDialError> {
        Ok(())
    }
}

struct ScriptedTokens;

impl VertexAccessTokenSource for ScriptedTokens {
    type Error = VertexTokenError;

    fn resolve<'source>(
        &'source self,
        _credentials: Option<&'source Json>,
        _project_id: Option<&'source str>,
    ) -> VertexTokenFuture<'source> {
        Box::pin(async {
            Ok((
                SecretString::from("probe-token"),
                String::from("probe-project"),
            ))
        })
    }
}

// the production adapter hands every core failure to the health probe error unchanged
#[tokio::test]
async fn realtime_probe_reports_the_core_failure() {
    let tokens: Arc<VertexTokenSource> = Arc::new(ScriptedTokens);
    let probe = RealtimeProbe {
        stored: Arc::new(CredentialSnapshot::default()),
        metadata: Arc::new(ModelCatalog::default()),
        azure: Arc::new(UnusedAzure),
        vertex: Arc::new(VertexLiveProbe {
            catalog: Arc::new(ModelCatalog::default()),
            settings: VertexSettings::default(),
            tokens,
        }),
        dialer: Arc::new(UnusedDialer),
        verification: Verification::Enabled,
        settings: RealtimeProbeSettings {
            azure_realtime_protocol: None,
            credential_timeout_seconds: 20.0_f64,
            max_message_size_bytes: 1024,
        },
    };
    let call = RealtimeCall {
        model: String::from("realtime-a"),
        custom_llm_provider: String::from("cohere"),
        api_base: None,
        api_key: None,
        api_version: None,
        model_params: mode_parameters(&[]),
        logging: None,
    };
    let error = probe
        .probe(call)
        .await
        .expect_err("an unsupported provider fails");
    assert_eq!(error.to_string(), "Unsupported model: realtime-a");
}

// B1301's accessors, B1304's region and B1365's Live config over the injected catalog
#[tokio::test]
async fn vertex_live_probe_reads_its_parameters() {
    let catalog = Arc::new(ModelCatalog::default());
    let tokens: Arc<VertexTokenSource> = Arc::new(ScriptedTokens);
    let probe = VertexLiveProbe {
        catalog: Arc::clone(&catalog),
        settings: VertexSettings::default(),
        tokens: Arc::clone(&tokens),
    };
    assert!(Arc::ptr_eq(&probe.catalog(), &catalog));
    assert_eq!(
        probe.region("gemini-live-model", Some("europe-west4")),
        "europe-west4"
    );
    assert_eq!(probe.region("gemini-live-model", None), "us-central1");
    let credentials: Json =
        serde_json::from_str("{\"type\": \"service_account\"}").expect("the test JSON parses");
    let parameters = probe.parameters(&mode_parameters(&[
        ("vertex_project", text("case-project")),
        ("vertex_ai_location", text("europe-west4")),
        ("vertex_ai_credentials", credentials.clone()),
    ]));
    assert_eq!(parameters.project.as_deref(), Some("case-project"));
    assert_eq!(parameters.location.as_deref(), Some("europe-west4"));
    assert_eq!(parameters.credentials, Some(credentials));
    let source = probe.tokens();
    assert!(Arc::ptr_eq(&source, &tokens));
    let (token, project) = source
        .resolve(None, None)
        .await
        .expect("the scripted token resolves");
    assert_eq!(token.expose_secret(), "probe-token");
    assert_eq!(project, "probe-project");
    let endpoint = probe.live(
        &VertexProbeTarget {
            model: String::from("gemini-live-model"),
            project: String::from("case-project"),
            region: String::from("europe-west4"),
            access_token: SecretString::from("probe-token"),
        },
        None,
    );
    assert_eq!(
        endpoint.url,
        "wss://europe-west4-aiplatform.googleapis.com/ws/google.cloud.aiplatform.v1.LlmBidiService/BidiGenerateContent"
    );
    assert_eq!(
        endpoint.headers.get("Authorization").map(String::as_str),
        Some("Bearer probe-token")
    );
    assert_eq!(
        endpoint
            .headers
            .get("x-goog-user-project")
            .map(String::as_str),
        Some("case-project")
    );
}
