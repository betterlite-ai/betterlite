use std::sync::Arc;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

use crate::api::realtime_connect::token::{
    VertexAccessTokenSource, VertexTokenError, resolve_vertex_token,
};
use crate::api::vertex_chirp_protocol::is_vertex_speech_to_text_model;

use super::{
    RealtimeProbeDependencies, RealtimeProbeError, RealtimeProbeRequest, RealtimeProbeSettings,
    probe,
};

pub type VertexTokenSource = dyn VertexAccessTokenSource<Error = VertexTokenError>;

// B1301's safe parameter accessors, over the merged deployment parameters
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VertexProbeParameters {
    pub project: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<Json>,
}

// what B1365's Live config is built from once the token and project are resolved
#[derive(Debug, Clone)]
pub struct VertexProbeTarget {
    pub model: String,
    pub project: String,
    pub region: String,
    pub access_token: SecretString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexProbeEndpoint {
    pub url: String,
    pub headers: Headers,
}

// B1301, B1304, B1011 and B1365 behind one seam: core cannot depend on betterlite-vertex
pub trait VertexProbeConfig: Send + Sync {
    fn catalog(&self) -> Arc<ModelCatalog>;
    fn region(&self, model: &str, configured: Option<&str>) -> String;
    fn parameters(&self, params: &JsonObject) -> VertexProbeParameters;
    fn tokens(&self) -> Arc<VertexTokenSource>;
    fn live(&self, target: &VertexProbeTarget, api_base: Option<&str>) -> VertexProbeEndpoint;
}

// a Speech-to-Text model is rejected before any credential work; every other model probes its Live endpoint
pub(super) async fn health_check(
    request: &RealtimeProbeRequest,
    merged: &JsonObject,
    api_base: Option<&str>,
    settings: &RealtimeProbeSettings,
    dependencies: &RealtimeProbeDependencies<'_>,
) -> Result<(), RealtimeProbeError> {
    let vertex = dependencies.vertex;
    let catalog = vertex.catalog();
    if is_vertex_speech_to_text_model(&catalog, &request.model) {
        return Err(RealtimeProbeError::VertexSpeechModel {
            model: request.model.clone(),
        });
    }
    let parameters = vertex.parameters(merged);
    let region = vertex.region(&request.model, parameters.location.as_deref());
    let (access_token, project) = resolve_vertex_token(
        vertex.tokens().as_ref(),
        parameters.credentials.as_ref(),
        parameters.project.as_deref(),
        settings.credential_timeout_seconds,
    )
    .await?;
    let endpoint = vertex.live(
        &VertexProbeTarget {
            model: request.model.clone(),
            project,
            region,
            access_token,
        },
        api_base,
    );
    probe(dependencies, endpoint.url, endpoint.headers, settings).await
}
