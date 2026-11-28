use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_azure::azure_files::client::Opened;
use betterlite_azure::azure_files::client::SelectedClient;
use betterlite_azure::azure_files::client::auth::FileAuth;
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_files::{
    Operation as FileOperation, Reply as FileReply, call as file_call,
};
use betterlite_azure::batches::error::AzureBatchesError;
use betterlite_azure::batches::{
    Operation as BatchOperation, Reply as BatchReply, call as batch_call,
};
use betterlite_azure::endpoints::projection::ClientKind;
use betterlite_http::client_kind::ClientKind as SdkKind;
use betterlite_openai::batches::error::SdkError;
use betterlite_openai::batches::records::{BatchCreate, BatchPage, BatchQuery, BatchStatus};
use betterlite_openai::batches::sdk::BatchesSdk;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{CreatePurpose, FileCreate, FileUpload};
use betterlite_openai::files::{DEFAULT_TIMEOUT, FilesClient};
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::Builder;
use url::Url;

use super::fixture::{FileFake, Planned, leaf_error, parse};
use crate::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};

const FILE_JSON: &str = r#"{"id":"file-1","object":"file","bytes":5,"created_at":1700000000,"filename":"notes.txt","purpose":"batch","status":"processed"}"#;
const STATUS_ERROR: &str = r#"{"error":{"code":"not_found","message":"no such file","param":"file_id","type":"invalid_request_error"}}"#;
const BATCH_JSON: &str = r#"{"id":"batch_1","completion_window":"24h","created_at":1700000000,"endpoint":"/v1/embeddings","input_file_id":"file-1","object":"batch","status":"validating","cancelled_at":null,"cancelling_at":null,"completed_at":null,"error_file_id":null,"errors":null,"expired_at":null,"expires_at":null,"failed_at":null,"finalizing_at":null,"in_progress_at":null,"metadata":null,"model":null,"output_file_id":null,"request_counts":null,"usage":null}"#;
const CREATE_TEXT: &str =
    r#"{"completion_window":"24h","endpoint":"/v1/embeddings","input_file_id":"file-1"}"#;

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the runtime builds")
        .block_on(future)
}

fn opened() -> Opened {
    let client = SelectedClient::new(
        FilesClient {
            api_key: SecretString::from("key-1"),
            base_url: Url::parse("https://fixture.test/openai/").expect("a base url"),
            organization: None,
            timeout: DEFAULT_TIMEOUT,
            max_retries: 0,
        },
        &FileAuth::ApiKey(SecretString::from("key-1")),
        Some(String::from("2024-10-21")),
        ClientKind::AzureOpenAi,
    )
    .expect("the fixture client builds");
    Opened {
        client,
        is_async: false,
    }
}

fn upload() -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: Some(String::from("notes.txt")),
            content_type: None,
            data: b"plain".to_vec(),
        },
        purpose: CreatePurpose::Batch,
        expires_after: None,
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    }
}

fn files_rows() {
    let fake = FileFake::new(Planned::json(200, FILE_JSON));
    let opened = opened();
    let data = upload();
    let result = block_on(dispatch(AzureResourceOperation::Files(Box::pin(
        file_call(&opened, &fake, FileOperation::Create(&data)),
    ))))
    .expect("the create reaches its leaf");
    let AzureResourceResult::Files(reply) = result else {
        panic!("the files dispatch returns a file reply");
    };
    let FileReply::Object(object) = reply else {
        panic!("a create returns a file object");
    };
    assert_eq!(object.id, "file-1");
    assert_eq!(object.bytes, 5);
    let seen = fake.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].url,
        "https://fixture.test/openai/files?api-version=2024-10-21"
    );
}

fn files_failure_row() {
    let fake = FileFake::new(Planned::json(404, STATUS_ERROR));
    let opened = opened();
    let data = upload();
    let error = leaf_error(block_on(dispatch(AzureResourceOperation::Files(Box::pin(
        file_call(&opened, &fake, FileOperation::Create(&data)),
    )))));
    assert!(matches!(
        error,
        AzureResourceError::Files(AzureFilesError::Files {
            source: FilesError::Status(_)
        })
    ));
}

struct Sdk {
    answer: Vec<u8>,
    failure: Option<u16>,
    page: BatchPage,
    calls: Mutex<Vec<String>>,
}

impl Sdk {
    fn recording(answer: &str) -> Self {
        Self {
            answer: answer.as_bytes().to_vec(),
            failure: None,
            page: parse(r#"{"data":[],"has_more":false}"#),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(status: u16) -> Self {
        Self {
            answer: Vec::new(),
            failure: Some(status),
            page: parse(r#"{"data":[],"has_more":false}"#),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("the fixture holds the calls")
            .clone()
    }

    fn reply(&self) -> Result<Vec<u8>, SdkError> {
        self.failure.map_or_else(
            || Ok(self.answer.clone()),
            |status| {
                Err(SdkError::Status {
                    status,
                    error_type: Some(String::from("server_error")),
                    param: None,
                    code: None,
                    message: String::from("the fixture failed"),
                })
            },
        )
    }

    fn record(&self, call: String) -> (Result<Vec<u8>, SdkError>, Result<BatchPage, SdkError>) {
        self.calls
            .lock()
            .expect("the fixture holds the calls")
            .push(call);
        (self.reply(), self.reply().map(|_answer| self.page.clone()))
    }
}

impl BatchesSdk for Sdk {
    type Page = BatchPage;

    fn kind(&self) -> SdkKind {
        SdkKind::Sync
    }

    fn create(
        &self,
        request: &BatchCreate,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.record(format!("create:{request:?}")).0)
    }

    fn retrieve(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.record(format!("retrieve:{request:?}")).0)
    }

    fn cancel(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.record(format!("cancel:{request:?}")).0)
    }

    fn list(
        &self,
        after: Option<&str>,
        limit: Option<i64>,
    ) -> impl Future<Output = Result<BatchPage, SdkError>> + Send {
        ready(self.record(format!("list:{after:?}:{limit:?}")).1)
    }
}

fn batches_rows() {
    let sdk = Sdk::recording(BATCH_JSON);
    let create: BatchCreate = parse(CREATE_TEXT);
    let result = block_on(dispatch(AzureResourceOperation::Batches(Box::pin(
        batch_call(&sdk, BatchOperation::Create(&create)),
    ))))
    .expect("the create reaches its leaf");
    let AzureResourceResult::Batches(BatchReply::Batch(batch)) = result else {
        panic!("a create returns the batch");
    };
    assert_eq!(batch.id, "batch_1");
    assert_eq!(batch.status, BatchStatus::Validating);
    assert_eq!(sdk.calls().len(), 1);
    assert!(sdk.calls()[0].starts_with("create:"));

    let listing = Sdk::recording(BATCH_JSON);
    let page = block_on(dispatch(AzureResourceOperation::Batches(Box::pin(
        batch_call(
            &listing,
            BatchOperation::List {
                after: Some("batch_0"),
                limit: Some(20),
            },
        ),
    ))));
    assert!(matches!(
        page,
        Ok(AzureResourceResult::Batches(BatchReply::Page(_)))
    ));
    assert_eq!(
        listing.calls(),
        vec![String::from("list:Some(\"batch_0\"):Some(20)")]
    );
}

fn batches_failure_row() {
    let sdk = Sdk::failing(500);
    let create: BatchCreate = parse(CREATE_TEXT);
    let error = leaf_error(block_on(dispatch(AzureResourceOperation::Batches(
        Box::pin(batch_call(&sdk, BatchOperation::Create(&create))),
    ))));
    assert!(matches!(
        error,
        AzureResourceError::Batches(AzureBatchesError::Sdk {
            source: SdkError::Status { status: 500, .. }
        })
    ));
}

pub(super) fn check() {
    files_rows();
    files_failure_row();
    batches_rows();
    batches_failure_row();
}
