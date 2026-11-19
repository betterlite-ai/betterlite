use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_error::proxy_exception::ProxyException;

use crate::auth_resource_grants::fallback::{
    ACOMPLETION_ROUTE, FallbackRequest, FallbackRoute, FallbackRouteError, PROBE_CONTENT,
    PreparedProvider, is_valid_fallback_model,
};
use crate::auth_resource_grants::tests::support::proxy_exception;

struct RecordingRoute {
    requests: Mutex<Vec<FallbackRequest>>,
    failure: Option<ProxyException>,
    polled: Arc<AtomicBool>,
}

impl RecordingRoute {
    fn new() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            failure: None,
            polled: Arc::new(AtomicBool::new(false)),
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            failure: Some(proxy_exception(message)),
            ..Self::new()
        }
    }

    fn requests(&self) -> Vec<FallbackRequest> {
        self.requests
            .lock()
            .expect("the route requests lock")
            .clone()
    }

    fn polled(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.polled)
    }
}

#[async_trait]
impl FallbackRoute for RecordingRoute {
    async fn route(
        &self,
        request: &FallbackRequest,
    ) -> Result<PreparedProvider, FallbackRouteError> {
        self.requests
            .lock()
            .expect("the route requests lock")
            .push(request.clone());
        if let Some(exception) = &self.failure {
            return Err(FallbackRouteError::from(Box::new(exception.clone())));
        }
        let polled = Arc::clone(&self.polled);
        Ok(PreparedProvider(Box::pin(async move {
            polled.store(true, Ordering::Release);
        })))
    }
}

#[tokio::test]
async fn fallback_probe_exact_payload() {
    let route = RecordingRoute::new();
    is_valid_fallback_model("claude-sonnet-4-5", Some("anthropic/claude"), &route)
        .await
        .expect("the prepared route validates the fallback");
    let requests = route.requests();
    assert_eq!(requests.len(), 1);
    let probe = &requests[0];
    assert_eq!(probe.model, "claude-sonnet-4-5");
    assert_eq!(probe.route_type, ACOMPLETION_ROUTE);
    assert_eq!(probe.user_model.as_deref(), Some("anthropic/claude"));
    assert_eq!(probe.messages.len(), 1);
    assert_eq!(probe.messages[0].role, "user");
    assert_eq!(probe.messages[0].content, PROBE_CONTENT);
    assert_eq!(PROBE_CONTENT, "Who was Alexander?");
}

#[tokio::test]
async fn fallback_probe_error() {
    let route = RecordingRoute::failing("model not found");
    let error = is_valid_fallback_model("missing-model", None, &route)
        .await
        .expect_err("the routing preparation failure propagates");
    assert_eq!(error.to_string(), "model not found");
    assert_eq!(route.requests().len(), 1);
    assert!(route.requests()[0].user_model.is_none());
}

#[tokio::test]
async fn fallback_probe_no_provider_call() {
    let route = RecordingRoute::new();
    let polled = route.polled();
    is_valid_fallback_model("gpt-4o", None, &route)
        .await
        .expect("the prepared route validates the fallback");
    assert!(!polled.load(Ordering::Acquire));
}
