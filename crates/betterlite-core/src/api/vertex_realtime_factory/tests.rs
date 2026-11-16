#![cfg(test)]

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::api_boundaries::ApiLimits;
use crate::api::realtime_contracts::error::WebSocketError;
use crate::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use crate::api::vertex_chirp::factory::ChirpBackendFactory;
use crate::api::vertex_chirp::ids::UuidChirpIds;
use crate::api::vertex_chirp::target::{
    AccessTokenError, AccessTokenFuture, AccessTokenResolver, SpeechStreamingTarget,
};
use crate::api::vertex_location::VertexLocationError;

use super::{
    VertexLiveTarget, VertexRealtimeConfig, VertexRealtimeError, VertexRealtimeInput,
    VertexRegionPolicy, vertex_realtime_config,
};

const TEST_TOKEN: &str = "ya29.test-token";

fn input<'input>(
    model: &'input str,
    project: &'input str,
    location: Option<&'input str>,
    catalog: &Arc<ModelCatalog>,
) -> VertexRealtimeInput<'input> {
    VertexRealtimeInput {
        model,
        project,
        location,
        access_token: SecretString::from(TEST_TOKEN.to_owned()),
        resolve_access_token: Arc::new(UnresolvedToken),
        catalog: Arc::clone(catalog),
        limits: ApiLimits::default(),
        ids: Box::new(UuidChirpIds::default()),
    }
}

#[test]
fn chirp_selection() {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let config = vertex_realtime_config(
        input("chirp_2", "my-project", Some("europe-west4"), &catalog),
        &PassThroughRegion,
        UnusedFactory,
    )
    .expect("the Speech-to-Text config builds");
    let VertexRealtimeConfig::Chirp(config) = config else {
        panic!("chirp_2 must select the Speech-to-Text backend");
    };
    assert_eq!(
        config
            .complete_url(None, "chirp_2")
            .expect("the url builds"),
        "europe-west4-speech.googleapis.com"
    );
    let live = vertex_realtime_config(
        input("chirp", "my-project", None, &catalog),
        &PassThroughRegion,
        UnusedFactory,
    )
    .expect("the Live target builds");
    assert!(
        matches!(live, VertexRealtimeConfig::Live(_)),
        "a model without the Speech-to-Text endpoints must select Vertex Live"
    );
}

#[test]
fn live_region_selection() {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let default_target = live_target(None, &catalog);
    assert_eq!(default_target.region, "policy-default");
    assert_eq!(
        live_target(Some("asia-east1"), &catalog).region,
        "policy-asia-east1"
    );
    assert_eq!(live_target(Some(""), &catalog).region, "policy-default");
    assert_eq!(default_target.project, "my-project");
    assert_eq!(default_target.model, "gemini-2.5-flash");
    assert_eq!(default_target.access_token.expose_secret(), TEST_TOKEN);
}

#[test]
fn invalid_region_is_rejected() {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let outcome = vertex_realtime_config(
        input(
            "gemini-2.5-flash",
            "my-project",
            Some("evil.example.com/x"),
            &catalog,
        ),
        &PassThroughRegion,
        UnusedFactory,
    );
    let Err(error) = outcome else {
        panic!("a region that is not a Vertex location must be rejected");
    };
    assert!(matches!(
        error,
        VertexRealtimeError::Target(VertexLocationError::Invalid)
    ));
}

#[test]
fn invalid_project_is_rejected() {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    assert!(matches!(
        project_outcome("", &catalog),
        VertexRealtimeError::Target(VertexLocationError::Project { .. })
    ));
    assert!(matches!(
        project_outcome("../other", &catalog),
        VertexRealtimeError::Target(VertexLocationError::Project { .. })
    ));
    assert!(matches!(
        project_outcome("a/b", &catalog),
        VertexRealtimeError::Target(VertexLocationError::Project { .. })
    ));
}

fn live_target(location: Option<&str>, catalog: &Arc<ModelCatalog>) -> Box<VertexLiveTarget> {
    let config = vertex_realtime_config(
        input("gemini-2.5-flash", "my-project", location, catalog),
        &PassThroughRegion,
        UnusedFactory,
    )
    .expect("the Live target builds");
    let VertexRealtimeConfig::Live(target) = config else {
        panic!("a Gemini Live model must select Vertex Live");
    };
    target
}

fn project_outcome(project: &str, catalog: &Arc<ModelCatalog>) -> VertexRealtimeError {
    let outcome = vertex_realtime_config(
        input("gemini-2.5-flash", project, None, catalog),
        &PassThroughRegion,
        UnusedFactory,
    );
    let Err(error) = outcome else {
        panic!("an unsafe project must be rejected");
    };
    error
}

struct PassThroughRegion;

impl VertexRegionPolicy for PassThroughRegion {
    fn region_for(
        &self,
        _catalog: &ModelCatalog,
        _model: &str,
        configured: Option<&str>,
    ) -> String {
        configured.map_or_else(
            || "policy-default".to_owned(),
            |region| format!("policy-{region}"),
        )
    }
}

struct UnresolvedToken;

impl AccessTokenResolver for UnresolvedToken {
    fn resolve_access_token(&self) -> AccessTokenFuture<'_> {
        Box::pin(async { Err(AccessTokenError) })
    }
}

struct UnusedFactory;

impl ChirpBackendFactory for UnusedFactory {
    type Backend = UnusedBackend;

    fn open(&self, _target: SpeechStreamingTarget) -> UnusedBackend {
        UnusedBackend
    }
}

struct UnusedBackend;

#[async_trait]
impl RealtimeBackend for UnusedBackend {
    async fn send(&mut self, _message: RealtimeMessage) -> Result<(), WebSocketError> {
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Err(WebSocketError {
            message: "the unused backend never receives".to_owned(),
        })
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        Ok(())
    }
}
