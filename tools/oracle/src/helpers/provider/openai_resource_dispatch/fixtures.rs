use std::future::{Future, ready};

use betterlite_http::client_kind::ClientKind;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::batches::error::SdkError as BatchSdkError;
use betterlite_openai::batches::records::{BatchCreate, BatchPage, BatchQuery};
use betterlite_openai::batches::sdk::{BatchesSdk, ClientConstructor, SdkArguments};
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use betterlite_openai::fine_tuning::client::{
    BuildError, ClientArgs, ClientBuilder, JobsApi, SdkError as JobSdkError,
};
use betterlite_openai::fine_tuning::job::FineTuneCreate;
use futures_util::StreamExt as _;
use futures_util::stream::iter;
use http::HeaderMap;
use serde_json::Map;

pub(super) struct Fixture {
    pub(super) status: u16,
    pub(super) body: Vec<u8>,
}

const FAILURE: &str = "the fixture upstream failed";

impl Fixture {
    fn answer(&self) -> Result<Vec<u8>, BatchSdkError> {
        if (200..300).contains(&self.status) {
            return Ok(self.body.clone());
        }
        Err(BatchSdkError::Status {
            status: self.status,
            error_type: None,
            param: None,
            code: None,
            message: String::from(FAILURE),
        })
    }
}

impl FileTransport for Fixture {
    fn send_buffered(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        }))
    }

    async fn send_stream(&self, request: OutboundRequest) -> Result<StreamReply, TransportError> {
        let buffered = self.send_buffered(request).await?;
        let chunks: Vec<Result<Vec<u8>, HttpError>> = vec![Ok(buffered.body)];
        Ok(StreamReply {
            status: buffered.status,
            headers: buffered.headers,
            body: iter(chunks).boxed(),
        })
    }
}

impl BatchesSdk for Fixture {
    type Page = BatchPage;

    fn kind(&self) -> ClientKind {
        ClientKind::Sync
    }

    fn create(
        &self,
        _request: &BatchCreate,
    ) -> impl Future<Output = Result<Vec<u8>, BatchSdkError>> + Send {
        ready(self.answer())
    }

    fn retrieve(
        &self,
        _request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, BatchSdkError>> + Send {
        ready(self.answer())
    }

    fn cancel(
        &self,
        _request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, BatchSdkError>> + Send {
        ready(self.answer())
    }

    fn list(
        &self,
        _after: Option<&str>,
        _limit: Option<i64>,
    ) -> impl Future<Output = Result<BatchPage, BatchSdkError>> + Send {
        let page = BatchPage {
            data: Vec::new(),
            has_more: None,
            provider_fields: Map::new(),
        };
        ready(Ok(page))
    }
}

impl JobsApi for Fixture {
    fn is_async(&self) -> bool {
        false
    }

    fn create(&self, _data: &FineTuneCreate) -> Result<Vec<u8>, JobSdkError> {
        self.answer().map_err(|_failure| JobSdkError {
            status: self.status,
            error_type: None,
            param: None,
            code: None,
            message: String::from(FAILURE),
        })
    }

    fn list(&self, _after: Option<&str>, _limit: Option<i64>) -> Result<Vec<u8>, JobSdkError> {
        Ok(self.body.clone())
    }

    fn retrieve(&self, _job_id: &str) -> Result<Vec<u8>, JobSdkError> {
        Ok(self.body.clone())
    }

    fn cancel(&self, _job_id: &str) -> Result<Vec<u8>, JobSdkError> {
        Ok(self.body.clone())
    }
}

// Both SDK seams get their client supplied, so neither seam ever builds one
pub(super) struct NoClient;

impl ClientConstructor for NoClient {
    type Client = Fixture;

    fn construct(
        &self,
        _arguments: &SdkArguments,
        _kind: ClientKind,
    ) -> Result<Option<Fixture>, BatchSdkError> {
        Ok(None)
    }
}

impl ClientBuilder for NoClient {
    fn build(
        &self,
        _is_async: bool,
        _args: &ClientArgs,
    ) -> Result<Option<Box<dyn JobsApi>>, BuildError> {
        Ok(None)
    }
}
