use std::future::pending;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;

use crate::deployment_catalog::types::Candidates;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::lar1_routing::health::HealthyDeployments;
use crate::lar1_routing::strategy::Lar1Strategy;
use crate::lar1_routing::thresholds::{ThresholdArgs, Thresholds};
use crate::router_core::error::RoutingError;

pub(super) fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

pub(super) fn strategy_with(low: &str, medium: &str, high: &str) -> Lar1Strategy {
    Lar1Strategy {
        thresholds: Thresholds {
            low: dec(low),
            medium: dec(medium),
            high: dec(high),
        },
    }
}

pub(super) fn args(low: Option<&str>, medium: Option<&str>, high: Option<&str>) -> ThresholdArgs {
    ThresholdArgs {
        low: low.map(dec),
        medium: medium.map(dec),
        high: high.map(dec),
    }
}

pub(super) fn spec(id: &str, kind: Option<&str>) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text("openai/probe".to_owned()));
    let mut model_info = JsonObject::new();
    model_info.insert("id".to_owned(), Json::Text(id.to_owned()));
    if let Some(kind_name) = kind {
        model_info.insert("type".to_owned(), Json::Text(kind_name.to_owned()));
    }
    DeploymentSpec::new(
        "probe-group".to_owned(),
        params,
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

pub(super) fn typed_pool() -> Vec<DeploymentSpec> {
    vec![
        spec("cloud-smart-1", Some("cloud-smart")),
        spec("cloud-fast-1", Some("cloud-fast")),
        spec("local-1", Some("local")),
        spec("deep-1", Some("deep")),
    ]
}

pub(super) struct Group(pub(super) Vec<DeploymentSpec>);

pub(super) struct Single(pub(super) DeploymentSpec);

pub(super) struct Failing;

pub(super) struct Pending;

#[async_trait]
impl HealthyDeployments for Group {
    async fn healthy_deployments(
        &self,
        _model: &str,
        _specific_deployment: bool,
    ) -> Result<Candidates, RoutingError> {
        Ok(Candidates::Pool(self.0.clone()))
    }
}

#[async_trait]
impl HealthyDeployments for Single {
    async fn healthy_deployments(
        &self,
        _model: &str,
        _specific_deployment: bool,
    ) -> Result<Candidates, RoutingError> {
        Ok(Candidates::Single(Box::new(self.0.clone())))
    }
}

#[async_trait]
impl HealthyDeployments for Failing {
    async fn healthy_deployments(
        &self,
        _model: &str,
        _specific_deployment: bool,
    ) -> Result<Candidates, RoutingError> {
        Err(RoutingError::Cancelled)
    }
}

#[async_trait]
impl HealthyDeployments for Pending {
    async fn healthy_deployments(
        &self,
        _model: &str,
        _specific_deployment: bool,
    ) -> Result<Candidates, RoutingError> {
        pending().await
    }
}

pub(super) async fn selected(
    strategy: &Lar1Strategy,
    router: Option<&dyn HealthyDeployments>,
    metadata: Option<&[u8]>,
    specific_deployment: bool,
) -> Result<Option<String>, RoutingError> {
    let found = strategy
        .select_deployment(router, metadata, specific_deployment, "probe-group")
        .await?;
    Ok(found.and_then(|deployment| deployment.id().map(str::to_owned)))
}

#[derive(Clone)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    // each log line without its level and target
    pub(super) fn messages(&self) -> String {
        let text = String::from_utf8(self.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = text
            .lines()
            .map(|line| line.split_once(": ").map_or(line, |(_, message)| message))
            .collect();
        lines.join("|")
    }
}

pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured(Arc::new(Mutex::new(Vec::new())));
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}
