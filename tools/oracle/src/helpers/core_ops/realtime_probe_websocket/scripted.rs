use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::realtime_backend_transport::dial::RealtimeSsl;
use betterlite_core::api::realtime_connect::token::{
    VertexAccessTokenSource, VertexTokenError, VertexTokenFuture,
};
use betterlite_core::api::realtime_connection_plan::azure::AzureAdTokens;
use betterlite_core::api::realtime_connection_plan::vertex::{
    VertexProbeConfig, VertexProbeEndpoint, VertexProbeParameters, VertexProbeTarget,
    VertexTokenSource,
};
use betterlite_core::api::realtime_connection_plan::{
    ProbeConnection, ProbeDialError, RealtimeProbeDialer, RealtimeProbeError,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_vertex::endpoint_target::region::select_region;
use betterlite_vertex::vertex_live::policy::VertexLivePolicy;
use secrecy::SecretString;

use super::Scenario;

// the probe opens exactly one websocket and sends nothing on it
#[derive(Default)]
pub(super) struct RecordingDialer {
    connections: Mutex<Vec<ProbeConnection>>,
}

impl RecordingDialer {
    pub(super) fn take(&self) -> Option<ProbeConnection> {
        self.connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
    }
}

#[async_trait]
impl RealtimeProbeDialer for RecordingDialer {
    async fn probe(&self, connection: &ProbeConnection) -> Result<(), ProbeDialError> {
        self.connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(connection.clone());
        Ok(())
    }
}

pub(super) const fn ssl_kind(ssl: &RealtimeSsl) -> &'static str {
    match ssl {
        RealtimeSsl::Shared(_) => "shared",
        RealtimeSsl::Insecure => "insecure",
        RealtimeSsl::Disabled => "disabled",
    }
}

// B529's resolver, scripted with the token the case supplies
pub(super) struct ScriptedAzure {
    pub(super) token: Option<SecretString>,
}

impl AzureAdTokens for ScriptedAzure {
    fn token(&self, _params: &JsonObject) -> Result<Option<SecretString>, RealtimeProbeError> {
        Ok(self.token.clone())
    }
}

// B1301, B1304, B1011 and B1365 scripted from the case, with B1304's region selection kept real
pub(super) struct ScriptedVertex {
    catalog: Arc<ModelCatalog>,
    location: Option<String>,
    source: Arc<ScriptedTokens>,
}

impl ScriptedVertex {
    pub(super) fn new(scenario: &Scenario, catalog: Arc<ModelCatalog>) -> Self {
        Self {
            catalog,
            location: scenario.vertex_location.clone(),
            source: Arc::new(ScriptedTokens {
                token: SecretString::from(scenario.vertex_token.clone().unwrap_or_default()),
                project: scenario.vertex_project.clone().unwrap_or_default(),
            }),
        }
    }
}

impl VertexProbeConfig for ScriptedVertex {
    fn catalog(&self) -> Arc<ModelCatalog> {
        Arc::clone(&self.catalog)
    }

    fn region(&self, model: &str, configured: Option<&str>) -> String {
        select_region(&self.catalog, model, configured).region
    }

    fn parameters(&self, _params: &JsonObject) -> VertexProbeParameters {
        VertexProbeParameters {
            project: None,
            location: self.location.clone(),
            credentials: None,
        }
    }

    fn tokens(&self) -> Arc<VertexTokenSource> {
        Arc::clone(&self.source)
    }

    fn live(&self, target: &VertexProbeTarget, api_base: Option<&str>) -> VertexProbeEndpoint {
        let policy = VertexLivePolicy {
            project: target.project.clone(),
            region: target.region.clone(),
            access_token: target.access_token.clone(),
        };
        VertexProbeEndpoint {
            url: policy.complete_url(api_base),
            headers: policy.environment(&BTreeMap::new()),
        }
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
