use super::native::{SendError, seconds};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::fine_tuning_create::VertexCreateCall;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::fine_tuning::job::FineTuneJob;
use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::probe::AccessToken;
use betterlite_vertex::vertex_fine_tuning::execution::TuningTransport;
use betterlite_vertex::vertex_fine_tuning::execution::create::{
    TuningCreateRequest, create_tuning_job,
};
use betterlite_vertex::vertex_fine_tuning::records::request::OriginalHyperparameters;
use jiff::tz::TimeZone;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::future::ready;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use url::Position;

// the source's mock auth returns this project, so the native seam returns the same one
pub(crate) async fn create(
    call: VertexCreateCall,
    urls: Arc<Mutex<Vec<String>>>,
) -> Result<FineTuneJob, SendError> {
    let VertexCreateCall {
        data,
        original_hyperparameters,
        api_base,
        vertex_project,
        vertex_location,
        timeout,
        ..
    } = call;
    let original = OriginalHyperparameters {
        adapter_size: adapter_size(&original_hyperparameters),
    };
    let transport = transport(&api_base, urls)?;
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let local = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    let request = TuningCreateRequest {
        catalog: &catalog,
        data: &data,
        original_hyperparameters: &original,
        credentials: None,
        project: vertex_project.as_deref(),
        location: vertex_location.as_deref(),
        api_base: (!api_base.is_empty()).then_some(api_base.as_str()),
        local: &local,
        timeout: seconds(timeout).and_then(|value| Duration::try_from_secs_f64(value).ok()),
    };
    create_tuning_job(&transport, &request, |_, _| {
        ready(Ok::<AccessToken, CredentialCacheError>(AccessToken {
            token: SecretString::from("vertex-test".to_owned()),
            project: "mock-project-id".to_owned(),
        }))
    })
    .await
    .map_err(SendError::from)
}
// LiteLLM reads adapter_size from the caller's original map, and a falsy value never reaches Vertex
fn adapter_size(original: &Extra) -> Option<String> {
    match original.get("adapter_size")? {
        Json::Text(text) if !text.is_empty() => Some(text.clone()),
        Json::Number(number) if number.as_f64().is_some_and(|value| value != 0.0_f64) => {
            Some(number.to_string())
        }
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}
fn transport(
    api_base: &str,
    urls: Arc<Mutex<Vec<String>>>,
) -> Result<RecordingTransport, SendError> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).map_err(|error| {
            SendError::Setup {
                message: error.to_string(),
            }
        })?,
    );
    let inner = ProxiedClient::outbound(&policy).map_err(|error| SendError::Setup {
        message: error.to_string(),
    })?;
    let mut parsed = url::Url::parse(api_base).map_err(|error| SendError::Setup {
        message: error.to_string(),
    })?;
    parsed.set_path("");
    parsed.set_query(None);
    let origin = parsed.as_str().trim_end_matches('/').to_owned();
    Ok(RecordingTransport {
        inner,
        origin,
        urls,
    })
}
// the closed test transport records the regional URL B1384 resolved and redirects only its origin
pub(crate) struct RecordingTransport {
    inner: ProxiedClient,
    origin: String,
    urls: Arc<Mutex<Vec<String>>>,
}
impl TuningTransport for RecordingTransport {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.url.to_string());
        let inner = self.inner.clone();
        let origin = self.origin.clone();
        async move {
            let target =
                url::Url::parse(&format!("{origin}{}", &request.url[Position::BeforePath..]))
                    .map_err(|source| TransportError::RedirectUrl { source })?;
            let rewritten = OutboundRequest {
                method: request.method,
                url: target,
                query: request.query,
                headers: request.headers,
                body: request.body,
                deadline: request.deadline,
            };
            inner.send_buffered(rewritten).await
        }
    }
}
