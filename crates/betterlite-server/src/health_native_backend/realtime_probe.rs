use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::tls::Verification;
use betterlite_core::api::realtime_connection_plan::azure::AzureAdTokens;
use betterlite_core::api::realtime_connection_plan::metadata::RealtimeModelMetadata;
use betterlite_core::api::realtime_connection_plan::vertex::{
    VertexProbeConfig, VertexProbeEndpoint, VertexProbeParameters, VertexProbeTarget,
    VertexTokenSource,
};
use betterlite_core::api::realtime_connection_plan::{
    RealtimeProbeDependencies, RealtimeProbeDialer, RealtimeProbeError, RealtimeProbeRequest,
    RealtimeProbeSettings, realtime_health_check,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_sdk::call_wrapper::error::plain_error;
use betterlite_vertex::credential_sources::getters::VertexParams;
use betterlite_vertex::endpoint_target::region::select_region;
use betterlite_vertex::vertex_live::policy::VertexLivePolicy;
use secrecy::ExposeSecret as _;

use super::seams::RealtimeProbeCall;
use crate::health_probe_modes::calls::{HealthProbeReply, RealtimeCall};
use crate::health_probe_modes::error::HealthProbeError;

const VERTEX_PROJECT_KEY: &str = "vertex_project";
const VERTEX_AI_PROJECT_KEY: &str = "vertex_ai_project";
const VERTEX_LOCATION_KEY: &str = "vertex_location";
const VERTEX_AI_LOCATION_KEY: &str = "vertex_ai_location";
const VERTEX_CREDENTIALS_KEY: &str = "vertex_credentials";
const VERTEX_AI_CREDENTIALS_KEY: &str = "vertex_ai_credentials";

// B1127's entry, wired with every resolver the check reads; the proxy builds it once it has them
pub struct RealtimeProbe {
    pub stored: Arc<CredentialSnapshot>,
    pub metadata: Arc<ModelCatalog>,
    pub azure: Arc<dyn AzureAdTokens>,
    pub vertex: Arc<dyn VertexProbeConfig>,
    pub dialer: Arc<dyn RealtimeProbeDialer>,
    pub verification: Verification,
    pub settings: RealtimeProbeSettings,
}

#[async_trait]
impl RealtimeProbeCall for RealtimeProbe {
    async fn probe(&self, call: RealtimeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let request = RealtimeProbeRequest {
            model: call.model,
            custom_llm_provider: call.custom_llm_provider,
            api_key: call.api_key,
            api_base: call.api_base,
            api_version: call.api_version,
            realtime_protocol: None,
            model_params: call.model_params,
        };
        let metadata: &dyn RealtimeModelMetadata = self.metadata.as_ref();
        let dependencies = RealtimeProbeDependencies {
            stored: self.stored.as_ref(),
            metadata,
            azure: self.azure.as_ref(),
            vertex: self.vertex.as_ref(),
            dialer: self.dialer.as_ref(),
            verification: &self.verification,
        };
        realtime_health_check(&request, &self.settings, &dependencies)
            .await
            .map(|()| HealthProbeReply::default())
            .map_err(probe_failure)
    }
}

fn probe_failure(error: RealtimeProbeError) -> HealthProbeError {
    let message = error.to_string();
    HealthProbeError::Backend {
        class: plain_error(&message),
        message,
        source: Box::new(error),
    }
}

// B1301's safe accessors, B1304's region selection and B1365's Live config over one catalog
pub struct VertexLiveProbe {
    pub catalog: Arc<ModelCatalog>,
    pub settings: VertexSettings,
    pub tokens: Arc<VertexTokenSource>,
}

impl VertexProbeConfig for VertexLiveProbe {
    fn catalog(&self) -> Arc<ModelCatalog> {
        Arc::clone(&self.catalog)
    }

    fn region(&self, model: &str, configured: Option<&str>) -> String {
        select_region(&self.catalog, model, configured).region
    }

    // the deployment's own parameters first, then the loaded Vertex settings, as the safe getters read them
    fn parameters(&self, params: &JsonObject) -> VertexProbeParameters {
        let project = text(params, VERTEX_PROJECT_KEY);
        let ai_project = text(params, VERTEX_AI_PROJECT_KEY);
        let location = text(params, VERTEX_LOCATION_KEY);
        let ai_location = text(params, VERTEX_AI_LOCATION_KEY);
        let vertex = VertexParams {
            project: project.as_deref(),
            ai_project: ai_project.as_deref(),
            location: location.as_deref(),
            ai_location: ai_location.as_deref(),
            credentials: None,
            ai_credentials: None,
        };
        VertexProbeParameters {
            project: vertex
                .safe_get_vertex_ai_project(&self.settings)
                .map(str::to_owned),
            location: vertex
                .safe_get_vertex_ai_location(&self.settings)
                .map(str::to_owned),
            credentials: credential_document(params).or_else(|| {
                self.settings.credentials.as_ref().map(|configured| {
                    Json::Text(match configured {
                        ConfiguredCredentials::Text(value) => value.expose_secret().to_owned(),
                        ConfiguredCredentials::Source(source) => source.document(),
                    })
                })
            }),
        }
    }

    fn tokens(&self) -> Arc<VertexTokenSource> {
        Arc::clone(&self.tokens)
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

fn text(params: &JsonObject, key: &str) -> Option<String> {
    match params.get(key) {
        Some(Json::Text(value)) => Some(value.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

// the credential document or file path travels as the deployment wrote it; the token source reads both
fn credential_document(params: &JsonObject) -> Option<Json> {
    params
        .get(VERTEX_CREDENTIALS_KEY)
        .or_else(|| params.get(VERTEX_AI_CREDENTIALS_KEY))
        .cloned()
}
