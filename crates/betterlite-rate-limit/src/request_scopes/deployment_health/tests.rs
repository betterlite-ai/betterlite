#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use tracing::Level;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt;

use super::{DeploymentHealthError, DeploymentHealthPolicy, DeploymentHealthSource, DeploymentRef};

struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut output) = self.0.lock() else {
            return Err(io::Error::other("poisoned capture"));
        };
        output.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capturing() -> (DefaultGuard, Arc<Mutex<Vec<u8>>>) {
    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&output);
    let subscriber = fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(Level::TRACE)
        .with_writer(move || Captured(Arc::clone(&sink)))
        .finish();
    (set_default(subscriber), output)
}

fn captured(output: &Arc<Mutex<Vec<u8>>>) -> String {
    let bytes = output.lock().expect("the capture is not poisoned").clone();
    String::from_utf8(bytes).expect("the capture is UTF-8")
}

struct FixedSource {
    deployments: Option<Vec<DeploymentRef>>,
    counts: BTreeMap<String, i64>,
    asked: Mutex<Vec<String>>,
}

impl FixedSource {
    fn new(deployments: Option<Vec<DeploymentRef>>, counts: &[(&str, i64)]) -> Self {
        let counts = counts
            .iter()
            .map(|(id, count)| ((*id).to_owned(), *count))
            .collect();
        Self {
            deployments,
            counts,
            asked: Mutex::new(Vec::new()),
        }
    }

    fn asked(&self) -> Vec<String> {
        match self.asked.lock() {
            Ok(asked) => asked.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[async_trait::async_trait]
impl DeploymentHealthSource for FixedSource {
    async fn deployments_for_model(
        &self,
        _model: &str,
    ) -> Result<Option<Vec<DeploymentRef>>, DeploymentHealthError> {
        Ok(self.deployments.clone())
    }

    async fn failures_this_minute(
        &self,
        deployment_id: &str,
    ) -> Result<i64, DeploymentHealthError> {
        match self.asked.lock() {
            Ok(mut asked) => asked.push(deployment_id.to_owned()),
            Err(poisoned) => poisoned.into_inner().push(deployment_id.to_owned()),
        }
        Ok(self.counts.get(deployment_id).copied().unwrap_or(0))
    }
}

struct BrokenSource {
    deployments_fail: bool,
}

#[async_trait::async_trait]
impl DeploymentHealthSource for BrokenSource {
    async fn deployments_for_model(
        &self,
        _model: &str,
    ) -> Result<Option<Vec<DeploymentRef>>, DeploymentHealthError> {
        if self.deployments_fail {
            return Err(DeploymentHealthError::new("router unavailable"));
        }
        Ok(Some(vec![DeploymentRef {
            id: Some("deployment-1".to_owned()),
        }]))
    }

    async fn failures_this_minute(
        &self,
        _deployment_id: &str,
    ) -> Result<i64, DeploymentHealthError> {
        Err(DeploymentHealthError::new("counter unavailable"))
    }
}

fn deployment(id: Option<&str>) -> DeploymentRef {
    DeploymentRef {
        id: id.map(str::to_owned),
    }
}

async fn has_failures(source: &dyn DeploymentHealthSource, threshold: i64) -> bool {
    DeploymentHealthPolicy {
        source,
        error_threshold_per_minute: threshold,
    }
    .model_has_recent_failures("gpt-4")
    .await
}

#[tokio::test]
async fn no_router_means_no_failures() {
    let source = FixedSource::new(None, &[]);
    assert!(!has_failures(&source, 1).await);
    assert!(source.asked().is_empty());
}

#[tokio::test]
async fn an_empty_deployment_list_means_no_failures() {
    let source = FixedSource::new(Some(Vec::new()), &[]);
    assert!(!has_failures(&source, 1).await);
    assert!(source.asked().is_empty());
}

#[tokio::test]
async fn deployments_without_an_id_are_skipped() {
    let source = FixedSource::new(
        Some(vec![deployment(None), deployment(Some(""))]),
        &[("", 9)],
    );
    assert!(!has_failures(&source, 1).await);
    assert!(source.asked().is_empty());
}

#[tokio::test]
async fn a_whitespace_id_is_still_an_id() {
    let source = FixedSource::new(
        Some(vec![deployment(None), deployment(Some(" "))]),
        &[("", 9), (" ", 9)],
    );
    assert!(has_failures(&source, 1).await);
    assert_eq!(source.asked(), [" "]);
}

#[rstest]
#[case::under(1, 0, false)]
#[case::at(1, 1, false)]
#[case::over(1, 2, true)]
#[case::zero_threshold_at(0, 0, false)]
#[case::zero_threshold_over(0, 1, true)]
#[tokio::test]
async fn the_threshold_is_a_strict_greater_than(
    #[case] threshold: i64,
    #[case] count: i64,
    #[case] expected: bool,
) {
    let source = FixedSource::new(
        Some(vec![deployment(Some("deployment-1"))]),
        &[("deployment-1", count)],
    );
    assert_eq!(has_failures(&source, threshold).await, expected);
}

#[tokio::test]
async fn the_first_deployment_over_the_threshold_decides() {
    let source = FixedSource::new(
        Some(vec![
            deployment(Some("deployment-1")),
            deployment(Some("deployment-2")),
        ]),
        &[("deployment-1", 2), ("deployment-2", 5)],
    );
    assert!(has_failures(&source, 1).await);
    assert_eq!(source.asked(), ["deployment-1"]);
}

#[tokio::test]
async fn a_later_deployment_over_the_threshold_decides() {
    let source = FixedSource::new(
        Some(vec![
            deployment(Some("deployment-1")),
            deployment(Some("deployment-2")),
        ]),
        &[("deployment-1", 1), ("deployment-2", 2)],
    );
    assert!(has_failures(&source, 1).await);
    assert_eq!(source.asked(), ["deployment-1", "deployment-2"]);
}

#[tokio::test]
async fn a_failure_read_failure_enforces_limits() {
    let (guard, output) = capturing();
    let source = BrokenSource {
        deployments_fail: false,
    };
    assert!(has_failures(&source, 1).await);
    drop(guard);
    assert!(captured(&output).contains(
        "Error checking model failure status: counter unavailable, defaulting to enforce limits"
    ));
}

#[tokio::test]
async fn a_router_read_failure_enforces_limits() {
    let (guard, output) = capturing();
    let source = BrokenSource {
        deployments_fail: true,
    };
    assert!(has_failures(&source, 1).await);
    drop(guard);
    assert!(captured(&output).contains(
        "Error checking model failure status: router unavailable, defaulting to enforce limits"
    ));
}

#[tokio::test]
async fn a_deployment_over_the_threshold_is_logged() {
    let (guard, output) = capturing();
    let source = FixedSource::new(
        Some(vec![deployment(Some("deployment-1"))]),
        &[("deployment-1", 2)],
    );
    assert!(has_failures(&source, 1).await);
    drop(guard);
    assert!(captured(&output).contains(
        "[Dynamic Rate Limit] Deployment deployment-1 has 2 failures in current minute - enforcing rate limits for model gpt-4"
    ));
}

#[tokio::test]
async fn no_deployment_over_the_threshold_is_logged() {
    let (guard, output) = capturing();
    let source = FixedSource::new(
        Some(vec![deployment(Some("deployment-1"))]),
        &[("deployment-1", 1)],
    );
    assert!(!has_failures(&source, 1).await);
    drop(guard);
    assert!(captured(&output).contains(
        "[Dynamic Rate Limit] No failures detected for model gpt-4 - allowing dynamic exceeding"
    ));
}
