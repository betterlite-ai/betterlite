#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use crate::batches::BatchesApi;
use crate::batches::error::SdkError;
use crate::batches::records::{
    BatchCreate, BatchEndpoint, BatchPage, BatchQuery, CompletionWindow,
};
use crate::batches::sdk::{
    BatchesSdk, ClientConstructor, ClientKind, ClientSettings, Connection, SdkArguments,
};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Recorded {
    Create(BatchCreate),
    Retrieve(BatchQuery),
    Cancel(BatchQuery),
    List {
        after: Option<String>,
        limit: Option<u32>,
    },
}

#[derive(Debug, Clone)]
pub(super) enum Answer {
    Batch(String),
    Failure,
}

pub(super) type Calls = Arc<Mutex<Vec<Recorded>>>;
pub(super) type Built = Arc<Mutex<Vec<(SdkArguments, ClientKind)>>>;

#[derive(Debug, Clone)]
pub(super) struct FakeSdk {
    pub(super) kind: ClientKind,
    pub(super) calls: Calls,
    pub(super) answer: Answer,
}

#[derive(Debug)]
pub(super) struct FakeConstructor {
    pub(super) produce: bool,
    pub(super) calls: Calls,
    pub(super) built: Built,
    pub(super) answer: Answer,
}

pub(super) fn batch_with(id: &str, status: &str, extra: &str) -> String {
    format!(
        r#"{{"id":"{id}","object":"batch","endpoint":"/v1/embeddings","input_file_id":"file-1","completion_window":"24h","status":"{status}","created_at":1700000000{extra}}}"#
    )
}

pub(super) fn batch(id: &str, status: &str) -> String {
    batch_with(id, status, "")
}

pub(super) fn page() -> BatchPage {
    let body = format!(
        r#"{{"data":[{}],"has_more":false,"object":"list"}}"#,
        batch("batch_1", "completed")
    );
    serde_json::from_str(&body).unwrap()
}

pub(super) fn failure() -> SdkError {
    SdkError::Status {
        status: 429,
        error_type: Some(String::from("rate_limit_error")),
        param: Some(String::from("limit")),
        code: Some(String::from("rate_limited")),
        message: String::from("slow down"),
    }
}

impl FakeSdk {
    fn reply(&self) -> Result<Vec<u8>, SdkError> {
        match &self.answer {
            Answer::Batch(body) => Ok(body.clone().into_bytes()),
            Answer::Failure => Err(failure()),
        }
    }

    fn record(&self, call: Recorded) {
        self.calls.lock().unwrap().push(call);
    }
}

impl BatchesSdk for FakeSdk {
    type Page = BatchPage;

    fn kind(&self) -> ClientKind {
        self.kind
    }

    fn create(
        &self,
        request: &BatchCreate,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        self.record(Recorded::Create(request.clone()));
        ready(self.reply())
    }

    fn retrieve(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        self.record(Recorded::Retrieve(request.clone()));
        ready(self.reply())
    }

    fn cancel(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        self.record(Recorded::Cancel(request.clone()));
        ready(self.reply())
    }

    fn list(
        &self,
        after: Option<&str>,
        limit: Option<u32>,
    ) -> impl Future<Output = Result<BatchPage, SdkError>> + Send {
        self.record(Recorded::List {
            after: after.map(str::to_owned),
            limit,
        });
        ready(match self.answer {
            Answer::Batch(_) => Ok(page()),
            Answer::Failure => Err(failure()),
        })
    }
}

impl ClientConstructor for FakeConstructor {
    type Client = FakeSdk;

    fn construct(&self, arguments: &SdkArguments, kind: ClientKind) -> Option<FakeSdk> {
        self.built.lock().unwrap().push((arguments.clone(), kind));
        self.produce.then(|| FakeSdk {
            kind,
            calls: Arc::clone(&self.calls),
            answer: self.answer.clone(),
        })
    }
}

pub(super) struct Fixture {
    pub(super) api: BatchesApi<FakeConstructor>,
    pub(super) calls: Calls,
    pub(super) built: Built,
}

pub(super) fn fixture(produce: bool, answer: Answer) -> Fixture {
    let calls = Calls::default();
    let built = Built::default();
    let constructor = FakeConstructor {
        produce,
        calls: Arc::clone(&calls),
        built: Arc::clone(&built),
        answer,
    };
    Fixture {
        api: BatchesApi { constructor },
        calls,
        built,
    }
}

pub(super) fn supplied(fixture: &Fixture, kind: ClientKind, answer: Answer) -> FakeSdk {
    FakeSdk {
        kind,
        calls: Arc::clone(&fixture.calls),
        answer,
    }
}

pub(super) fn connection(is_async: bool, client: Option<FakeSdk>) -> Connection<FakeSdk> {
    Connection {
        is_async,
        settings: ClientSettings::default(),
        client,
    }
}

pub(super) fn create_request() -> BatchCreate {
    BatchCreate {
        completion_window: CompletionWindow::Day,
        endpoint: BatchEndpoint::Embeddings,
        input_file_id: String::from("file-1"),
        metadata: None,
        output_expires_after: None,
        extra_headers: None,
        extra_body: None,
        timeout: None,
    }
}

pub(super) fn query() -> BatchQuery {
    BatchQuery {
        batch_id: String::from("batch_1"),
        extra_headers: None,
        extra_body: None,
        timeout: None,
    }
}
