#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::middleware;
use betterlite_config::sections::auth_route_patterns::Settings;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use tower::ServiceExt as _;

use crate::auth::auth_route_patterns::{AuthSettings, stage};
use crate::debug_routes::cache_memory::{CacheSnapshot, CacheSource};
use crate::debug_routes::otel_spans::{FinishedSpanSource, RecordedSpan};
use crate::debug_routes::{DebugRoutesState, router as debug_router};

pub(crate) const MASTER_KEY: &str = "sk-master";

pub(crate) struct StaticCache {
    snapshot: CacheSnapshot,
}

impl CacheSource for StaticCache {
    fn snapshot(&self) -> CacheSnapshot {
        self.snapshot.clone()
    }
}

pub(crate) struct StaticSpans {
    spans: Vec<RecordedSpan>,
}

impl FinishedSpanSource for StaticSpans {
    fn finished_spans(&self) -> Vec<RecordedSpan> {
        self.spans.clone()
    }
}

pub(crate) fn cache(
    cache_dict: BTreeMap<String, Json>,
    ttl_dict: BTreeMap<String, f64>,
) -> Arc<dyn CacheSource> {
    Arc::new(StaticCache {
        snapshot: CacheSnapshot {
            cache_dict,
            ttl_dict,
        },
    })
}

pub(crate) fn dict(keys: &[&str]) -> BTreeMap<String, Json> {
    keys.iter()
        .map(|key| ((*key).to_owned(), Json::Text("value".to_owned())))
        .collect()
}

pub(crate) fn ttl(keys: &[&str]) -> BTreeMap<String, f64> {
    keys.iter()
        .map(|key| ((*key).to_owned(), 1_700_000_000.5_f64))
        .collect()
}

pub(crate) fn state() -> DebugRoutesState {
    DebugRoutesState {
        user_api_key_cache: cache(BTreeMap::new(), BTreeMap::new()),
        user_key_object_cache: cache(BTreeMap::new(), BTreeMap::new()),
        llm_router_cache: None,
        proxy_logging_cache: cache(BTreeMap::new(), BTreeMap::new()),
        spans: None,
    }
}

pub(crate) fn state_with(
    user_api_key_cache: Arc<dyn CacheSource>,
    user_key_object_cache: Arc<dyn CacheSource>,
    llm_router_cache: Option<Arc<dyn CacheSource>>,
    proxy_logging_cache: Arc<dyn CacheSource>,
) -> DebugRoutesState {
    DebugRoutesState {
        user_api_key_cache,
        user_key_object_cache,
        llm_router_cache,
        proxy_logging_cache,
        spans: None,
    }
}

pub(crate) fn with_spans(state: DebugRoutesState, spans: Vec<RecordedSpan>) -> DebugRoutesState {
    DebugRoutesState {
        spans: Some(Arc::new(StaticSpans { spans })),
        ..state
    }
}

pub(crate) fn router(state: DebugRoutesState, profile: Option<&str>) -> Router {
    let mut environment: BTreeMap<String, String> = BTreeMap::new();
    if let Some(value) = profile {
        environment.insert("LITELLM_PROFILE".to_owned(), value.to_owned());
    }
    debug_router(state, &environment)
}

pub(crate) fn protected(router: Router) -> Router {
    let auth = AuthSettings {
        master_key: Some(SecretString::from(MASTER_KEY.to_owned())),
        routes: Settings {
            public_routes: Vec::new(),
        },
    };
    router.layer(middleware::from_fn_with_state(auth, stage))
}

pub(crate) async fn call(
    router: Router,
    method: &str,
    uri: &str,
    key: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = key {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let request = builder.body(Body::empty()).unwrap();
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}
