#![cfg(test)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::DeploymentSyncPort;

struct RecordingSync {
    calls: Arc<AtomicUsize>,
}

impl DeploymentSyncPort for RecordingSync {
    fn add_deployment(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
        })
    }
}

#[tokio::test]
async fn awaiting_add_deployment_reaches_the_implementation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let port: Arc<dyn DeploymentSyncPort> = Arc::new(RecordingSync {
        calls: Arc::clone(&calls),
    });
    port.add_deployment().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn awaiting_through_a_second_clone_reaches_the_same_implementation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let port: Arc<dyn DeploymentSyncPort> = Arc::new(RecordingSync {
        calls: Arc::clone(&calls),
    });
    let shared = Arc::clone(&port);
    shared.add_deployment().await;
    shared.add_deployment().await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
