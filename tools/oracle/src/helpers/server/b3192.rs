use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::HeaderMap;
use betterlite_config::sections::public_router_presets::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::public_endpoints::router_presets::catalog::PresetCatalog;
use betterlite_server::public_endpoints::router_presets::scorer::ComplexityScorerDefaults;
use betterlite_server::public_endpoints::router_presets::service::AutorouterPresetService;
use betterlite_server::public_endpoints::router_presets::{
    FUSE_PRESETS_PATH, router_presets_routes,
};
use betterlite_server::roi_github::transport::Transport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::Builder;
use tokio::task::yield_now;
use tower::ServiceExt as _;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "autorouter_presets")]
    Autorouter {
        url: String,
        local_only: bool,
        responses: Vec<RemoteResponse>,
        resolutions: u8,
    },
    #[serde(rename = "scorer_defaults")]
    ScorerDefaults,
    #[serde(rename = "fuse_presets")]
    FusePresets,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoteResponse {
    status: u16,
    body: String,
}

#[derive(Serialize)]
struct AutorouterAnswer<'presets> {
    presets: &'presets PresetCatalog,
    requests: usize,
    urls: Vec<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Autorouter {
                url,
                local_only,
                responses,
                resolutions,
            } => autorouter_answer(url, *local_only, responses, *resolutions),
            Input::ScorerDefaults => {
                let defaults = ComplexityScorerDefaults::shipped().or_config_error()?;
                Ok(serde_json::to_value(defaults)?)
            }
            Input::FusePresets => fuse_answer(),
        }
    }
}

// the route the proxy mounts, so the answer is the endpoint's own serialization
fn fuse_answer() -> HelperResult {
    let service = AutorouterPresetService::new(
        Script::new(&[]),
        Settings {
            local_only: true,
            url: String::new(),
        },
    );
    let router = router_presets_routes(Arc::new(service));
    let runtime = Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let request = Request::builder()
            .uri(FUSE_PRESETS_PATH)
            .body(Body::empty())
            .or_config_error()?;
        let response = router.oneshot(request).await.or_config_error()?;
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .or_config_error()?;
        Ok(serde_json::from_slice::<Value>(&body)?)
    })
}

fn autorouter_answer(
    url: &str,
    local_only: bool,
    responses: &[RemoteResponse],
    resolutions: u8,
) -> HelperResult {
    let script = Script::new(responses);
    let settings = Settings {
        local_only,
        url: url.to_owned(),
    };
    let service = AutorouterPresetService::new(script.clone(), settings);
    let runtime = Builder::new_current_thread().build()?;
    let presets = runtime.block_on(async {
        let mut resolved = service.presets().await.or_config_error()?;
        for _ in 1..resolutions {
            resolved = service.presets().await.or_config_error()?;
        }
        Ok::<_, Box<HelperError>>(resolved)
    })?;
    let urls = script.requests();
    let answer = AutorouterAnswer {
        presets: presets.as_ref(),
        requests: urls.len(),
        urls,
    };
    Ok(serde_json::to_value(answer)?)
}

#[derive(Clone, Default)]
struct Script {
    replies: Arc<Mutex<VecDeque<RemoteResponse>>>,
    urls: Arc<Mutex<Vec<String>>>,
}

impl Script {
    fn new(responses: &[RemoteResponse]) -> Self {
        let replies = VecDeque::from(responses.to_vec());
        Self {
            replies: Arc::new(Mutex::new(replies)),
            urls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<String> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for Script {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        yield_now().await;
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.url.to_string());
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match next {
            Some(response) => Ok(UpstreamResponse {
                status: response.status,
                headers: HeaderMap::new(),
                body: response.body.into_bytes(),
            }),
            None => Err(TransportError::Closed),
        }
    }
}
