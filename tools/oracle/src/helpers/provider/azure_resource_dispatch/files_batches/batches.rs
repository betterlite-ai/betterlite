use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_azure::batches::error::AzureBatchesError;
use betterlite_azure::batches::{Operation, Reply as BatchReply, call};
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::batches::error::SdkError;
use betterlite_openai::batches::records::{BatchCreate, BatchPage, BatchQuery};
use betterlite_openai::batches::sdk::BatchesSdk;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use serde_json::{Map, Value, json};

use super::super::support::{OTHER, failure, runtime, with_status};
use super::{Fixture, Step};

const CREATE_TEXT: &str =
    r#"{"completion_window":"24h","endpoint":"/v1/embeddings","input_file_id":"file-1"}"#;

pub(super) fn answer(step: &Step) -> HelperResult {
    let fixture = step.fixture();
    let runtime = runtime()?;
    let sdk = Sdk::new(fixture);
    let create: BatchCreate = serde_json::from_str(CREATE_TEXT).map_err(failure)?;
    let query = BatchQuery {
        batch_id: fixture.target_id.clone(),
        extra_headers: None,
        extra_body: None,
        timeout: None,
    };
    let operation = match step {
        Step::BatchCreate(_) => Operation::Create(&create),
        Step::BatchRetrieve(_) => Operation::Retrieve(&query),
        Step::BatchCancel(_) => Operation::Cancel(&query),
        Step::BatchList(_) => Operation::List {
            after: fixture.after.as_deref(),
            limit: fixture.limit,
        },
        Step::FileCreate(_)
        | Step::FileContent(_)
        | Step::FileRetrieve(_)
        | Step::FileList(_)
        | Step::FileDelete(_) => return Err(failure(OTHER)),
    };
    let result = runtime.block_on(dispatch(AzureResourceOperation::Batches(Box::pin(call(
        &sdk, operation,
    )))));
    match result {
        Ok(AzureResourceResult::Batches(reply)) => Ok(value(reply)),
        Ok(_) => Err(failure(OTHER)),
        Err(AzureResourceError::Batches(AzureBatchesError::Sdk { source })) => {
            Err(sdk_failure(&source))
        }
        Err(error) => Err(failure(error)),
    }
}

// The fixture SDK answers every call from the step's reply, recording the call it served
struct Sdk {
    answer: Vec<u8>,
    page: BatchPage,
    failure: Option<u16>,
    calls: Mutex<Vec<String>>,
}

impl Sdk {
    fn new(fixture: &Fixture) -> Self {
        let page = serde_json::from_str(&fixture.body).unwrap_or_else(|_error| BatchPage {
            data: Vec::new(),
            has_more: None,
            provider_fields: Map::new(),
        });
        Self {
            answer: fixture.body.clone().into_bytes(),
            page,
            failure: (fixture.status >= 400).then_some(fixture.status),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn reply(&self) -> Result<Vec<u8>, SdkError> {
        self.failure.map_or_else(
            || Ok(self.answer.clone()),
            |status| {
                Err(SdkError::Status {
                    status,
                    error_type: None,
                    param: None,
                    code: None,
                    message: String::from("the fixture failed"),
                })
            },
        )
    }

    fn record(&self, call: String) -> Result<Vec<u8>, SdkError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.reply()
    }
}

impl BatchesSdk for Sdk {
    type Page = BatchPage;

    fn kind(&self) -> ClientKind {
        ClientKind::Sync
    }

    fn create(
        &self,
        request: &BatchCreate,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.record(format!("create:{request:?}")))
    }

    fn retrieve(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.record(format!("retrieve:{request:?}")))
    }

    fn cancel(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.record(format!("cancel:{request:?}")))
    }

    fn list(
        &self,
        after: Option<&str>,
        limit: Option<i64>,
    ) -> impl Future<Output = Result<BatchPage, SdkError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("list:{after:?}:{limit:?}"));
        ready(self.reply().map(|_answer| self.page.clone()))
    }
}

fn value(reply: BatchReply) -> Value {
    match reply {
        BatchReply::Batch(batch) => json!({
            "id": batch.id,
            "status": serde_json::to_value(batch.status).unwrap_or(Value::Null),
        }),
        BatchReply::Page(page) => json!({
            "count": page.data.len(),
            "has_more": page.has_more.unwrap_or(false),
        }),
    }
}

fn sdk_failure(source: &SdkError) -> Box<HelperError> {
    let SdkError::Status { status, .. } = source else {
        return failure(source);
    };
    with_status(failure("the fixture failed"), *status)
}
