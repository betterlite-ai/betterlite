use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use betterlite_error::proxy_exception::ProxyException;

// route_request's probe route and body, which LiteLLM builds for every fallback target
pub const ACOMPLETION_ROUTE: &str = "acompletion";

pub const PROBE_CONTENT: &str = "Who was Alexander?";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackMessage {
    pub role: &'static str,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackRequest {
    pub model: String,
    pub messages: Vec<FallbackMessage>,
    pub route_type: &'static str,
    pub user_model: Option<String>,
}

// the provider call route_request prepared; polling it would send the probe request upstream
pub struct PreparedProvider(pub Pin<Box<dyn Future<Output = ()> + Send>>);

// B004's route_request as this check consumes it: preparation now, the provider coroutine returned unawaited
#[async_trait]
pub trait FallbackRoute: Send + Sync {
    async fn route(
        &self,
        request: &FallbackRequest,
    ) -> Result<PreparedProvider, FallbackRouteError>;
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct FallbackRouteError(#[from] Box<ProxyException>);

pub async fn is_valid_fallback_model(
    model: &str,
    user_model: Option<&str>,
    route: &dyn FallbackRoute,
) -> Result<(), FallbackRouteError> {
    let request = FallbackRequest {
        model: model.to_owned(),
        messages: vec![FallbackMessage {
            role: "user",
            content: PROBE_CONTENT.to_owned(),
        }],
        route_type: ACOMPLETION_ROUTE,
        user_model: user_model.map(str::to_owned),
    };
    // the returned coroutine is the provider call and is never polled
    let _prepared = route.route(&request).await?;
    Ok(())
}
