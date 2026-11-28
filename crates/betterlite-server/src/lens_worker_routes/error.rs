use std::convert::Infallible;
use std::error::Error as StdError;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use betterlite_db::lens_repository::repository::LensRepositoryError;
use betterlite_db::lens_repository::update::LensUpdateError;
use betterlite_lens::sources::source_reader::SourceError;
use thiserror::Error;

use crate::lens_access::error::{LensAccessError, failure_text};
use crate::lens_admin_routes::body::{BodyFailure, Issue, QueryFailure};
use crate::lens_inference::analyze::AnalyzeError;
use crate::tracing_routes::router::DetailBody;

// the refusals the worker routes share, with LiteLLM's texts
#[derive(Debug, Error)]
pub(crate) enum LensWorkerError {
    #[error("Not authenticated")]
    NotAuthenticated,
    #[error("Worker credential is invalid or revoked")]
    InvalidCredential,
    #[error(transparent)]
    Access(#[from] LensAccessError),
    #[error(transparent)]
    Repository(#[from] LensRepositoryError),
    #[error("{0}")]
    Source(#[source] Box<dyn StdError + Send + Sync>),
    #[error(transparent)]
    Analyze(#[from] AnalyzeError),
    #[error(transparent)]
    Body(#[from] BodyFailure),
    #[error(transparent)]
    Query(#[from] QueryFailure),
    #[error("This worker no longer owns the job")]
    NotOwned,
    #[error("Job was cancelled or reassigned")]
    JobMoved,
    #[error("Could not freeze the sample")]
    SampleNotFrozen,
    #[error("Execution is outside this job's sample")]
    OutsideSample,
    #[error("Upgrade the Lens worker using the current Connect worker command")]
    UpgradeWorker,
    #[error("Assign an analysis key to this worker in Lens setup")]
    NoAnalysisKey,
    #[error("Each run must have one assessment")]
    DuplicateRun,
    #[error("Assessment references a run outside this job")]
    RunOutside,
    #[error("Assessment references an unknown check")]
    CheckUnknown,
    #[error("Finding references evidence outside the job")]
    FindingOutside,
    #[error("Existing finding must belong to the same check")]
    ExistingCheck,
    #[error("Evidence quote does not match stored content")]
    EvidenceMismatch,
    #[error("the evidence names a run outside this job")]
    MissingRun,
}

// the sample freeze transform's only failure: the job left this worker between the read and the write
#[derive(Debug)]
pub(crate) struct JobMoved;

impl<E> From<SourceError<E>> for LensWorkerError
where
    E: StdError + Send + Sync + 'static,
{
    fn from(error: SourceError<E>) -> Self {
        Self::Source(Box::new(error))
    }
}

impl LensWorkerError {
    pub(crate) const fn status(&self) -> StatusCode {
        match self {
            Self::NotAuthenticated | Self::InvalidCredential => StatusCode::UNAUTHORIZED,
            Self::Access(error) => error.status(),
            Self::Repository(_) | Self::Source(_) | Self::MissingRun => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Analyze(error) => match StatusCode::from_u16(error.status()) {
                Ok(status) => status,
                Err(_invalid) => StatusCode::INTERNAL_SERVER_ERROR,
            },
            Self::Body(_)
            | Self::Query(_)
            | Self::DuplicateRun
            | Self::RunOutside
            | Self::CheckUnknown
            | Self::FindingOutside
            | Self::ExistingCheck
            | Self::EvidenceMismatch => StatusCode::UNPROCESSABLE_ENTITY,
            Self::OutsideSample => StatusCode::NOT_FOUND,
            Self::NotOwned
            | Self::JobMoved
            | Self::SampleNotFrozen
            | Self::UpgradeWorker
            | Self::NoAnalysisKey => StatusCode::CONFLICT,
        }
    }

    pub(crate) fn detail(&self) -> String {
        // a wrapped failure names its own cause chain; every other refusal is its own text
        if let Self::Access(error) = self {
            return failure_text(error);
        }
        if let Self::Repository(error) = self {
            return failure_text(error);
        }
        if let Self::Source(error) = self {
            return failure_text(error.as_ref());
        }
        if let Self::Analyze(error) = self {
            return error.detail();
        }
        self.to_string()
    }
}

// a lens transform that cannot fail: the update only carries the repository's own failure up
pub(crate) fn infallible_update(error: LensUpdateError<Infallible>) -> LensWorkerError {
    match error {
        LensUpdateError::Repository(source) => LensWorkerError::Repository(source),
        LensUpdateError::Transform(never) => match never {},
    }
}

// FastAPI's query validation issue, for a parameter that failed before the handler ran
pub(crate) fn query_refusal(kind: &'static str, field: &str, msg: &'static str) -> LensWorkerError {
    let issue = Issue {
        kind,
        loc: vec!["query".to_owned(), field.to_owned()],
        msg: msg.to_owned(),
    };
    LensWorkerError::Body(BodyFailure::Fields(vec![issue]))
}

// HTTPBearer's own refusal: detail "Not authenticated" plus the scheme in WWW-Authenticate
fn bearer_refusal() -> Response {
    let body = Json(DetailBody {
        detail: "Not authenticated".to_owned(),
    });
    let mut response = (StatusCode::UNAUTHORIZED, body).into_response();
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

impl IntoResponse for LensWorkerError {
    fn into_response(self) -> Response {
        let (status, detail) = match self {
            Self::NotAuthenticated => return bearer_refusal(),
            Self::Body(error) => return error.into_response(),
            Self::Query(error) => return error.into_response(),
            other @ (Self::InvalidCredential
            | Self::Access(_)
            | Self::Repository(_)
            | Self::Source(_)
            | Self::Analyze(_)
            | Self::NotOwned
            | Self::JobMoved
            | Self::SampleNotFrozen
            | Self::OutsideSample
            | Self::UpgradeWorker
            | Self::NoAnalysisKey
            | Self::DuplicateRun
            | Self::RunOutside
            | Self::CheckUnknown
            | Self::FindingOutside
            | Self::ExistingCheck
            | Self::EvidenceMismatch
            | Self::MissingRun) => (other.status(), other.detail()),
        };
        (status, Json(DetailBody { detail })).into_response()
    }
}

mod tests;
