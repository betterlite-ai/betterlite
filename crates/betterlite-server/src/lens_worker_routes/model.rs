use std::convert::Infallible;
use std::future::{Future, ready};
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, header};
use betterlite_auth::reservation_cost::ModelInfoSource;
use betterlite_lens::models::messages::{ModelRequest, ModelResult};
use betterlite_lens::sources::storage::Storage;
use betterlite_logging::clickhouse::clickhouse_logging::context::LensAnalysis;

use crate::lens_access::lookup::lens_repository;
use crate::lens_access::state::{LensDatabase, LensStorage};
use crate::lens_admin_routes::body::decode;
use crate::lens_billing::completion::Billing;
use crate::lens_billing::internal_request::{Incoming, MessagePrivacy};
use crate::lens_billing::key_validation::KeyResolver;
use crate::lens_inference::analyze::{AnalysisInput, InferencePorts, analyze};
use crate::lens_inference::charge::CostCalculator;
use crate::lens_inference::ports::{Clock, ModelCatalog};
use crate::lens_worker_routes::WorkerRoutesState;
use crate::lens_worker_routes::credential::worker_auth;
use crate::lens_worker_routes::error::LensWorkerError;
use crate::lens_worker_routes::lease::assigned;

// the analysis ports owned together, because InferencePorts only borrows and the state must be Clone
#[derive(Clone)]
pub struct Inference {
    pub(crate) billing: Arc<Billing>,
    pub(crate) keys: Arc<dyn KeyResolver>,
    pub(crate) models: Arc<dyn ModelCatalog>,
    pub(crate) prices: Arc<dyn ModelInfoSource>,
    pub(crate) costs: Arc<dyn CostCalculator>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) lens_analysis: LensAnalysis,
}

impl Inference {
    pub(crate) fn ports(&self) -> InferencePorts<'_> {
        InferencePorts {
            billing: self.billing.as_ref(),
            keys: self.keys.as_ref(),
            models: self.models.as_ref(),
            prices: self.prices.as_ref(),
            costs: self.costs.as_ref(),
            clock: self.clock.as_ref(),
            lens_analysis: &self.lens_analysis,
        }
    }
}

// the parts of the worker's request analyze rebuilds its in-process one from, read before the body
pub(crate) struct ModelContext {
    pub(crate) authorization: Option<HeaderValue>,
    pub(crate) incoming: Incoming,
}

impl<S> FromRequestParts<S> for ModelContext
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let authorization = parts.headers.get(header::AUTHORIZATION).cloned();
        let incoming = Incoming {
            scheme: parts.uri.scheme_str().map(str::to_owned),
            client: parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|connect| connect.0),
            x_forwarded_for: header_text(&parts.headers, "x-forwarded-for"),
            x_forwarded_proto: header_text(&parts.headers, "x-forwarded-proto"),
            host: header_text(&parts.headers, header::HOST.as_str()),
            privacy: MessagePrivacy::default(),
        };
        ready(Ok(Self {
            authorization,
            incoming,
        }))
    }
}

pub(crate) async fn model<Db, Store>(
    State(state): State<WorkerRoutesState<Db, Store>>,
    Path((lens_id, job_id)): Path<(String, String)>,
    context: ModelContext,
    body: Bytes,
) -> Result<Json<ModelResult>, LensWorkerError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let worker = worker_auth(&state.routes, context.authorization.as_ref()).await?;
    let request: ModelRequest = decode(&body)?;
    let now = state.routes.clock.now();
    let (lens, job) = assigned(&state.routes, &lens_id, &job_id, &worker, now).await?;
    let repository = lens_repository(&state.routes)?;
    let input = AnalysisInput {
        lens: &lens,
        job: &job,
        worker: &worker,
        body: &request,
        incoming: &context.incoming,
    };
    let result = analyze(repository, &state.inference.ports(), input).await?;
    Ok(Json(result))
}

// starlette decodes header bytes as latin-1, so every byte keeps the same char code
fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name).map(|value| {
        value
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect()
    })
}

mod tests;
