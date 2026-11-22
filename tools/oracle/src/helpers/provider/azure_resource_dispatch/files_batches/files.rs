use betterlite_azure::azure_files::client::auth::FileAuth;
use betterlite_azure::azure_files::client::{Opened, SelectedClient};
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_files::{Operation, Reply as FileReply, call};
use betterlite_azure::endpoints::projection::ClientKind;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{
    CreatePurpose, FileContentRequest, FileCreate, FileUpload,
};
use betterlite_openai::files::{DEFAULT_TIMEOUT, FilesClient};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use url::Url;

use super::super::support::{FileFake, OTHER, Reply, failure, runtime, with_status};
use super::{Fixture, Step};

pub(super) fn answer(step: &Step) -> HelperResult {
    let fixture = step.fixture();
    let runtime = runtime()?;
    let opened = opened(fixture)?;
    let fake = FileFake {
        reply: Reply::json(fixture.status, &fixture.body),
    };
    let data = create();
    let request = FileContentRequest {
        file_id: fixture.target_id.clone(),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    };
    let operation = match step {
        Step::FileCreate(_) => Operation::Create(&data),
        Step::FileContent(_) => Operation::Content(&request),
        Step::FileRetrieve(_) => Operation::Retrieve(&fixture.target_id),
        Step::FileList(_) => Operation::List(None),
        Step::FileDelete(_) => Operation::Delete(&fixture.target_id),
        Step::BatchCreate(_)
        | Step::BatchRetrieve(_)
        | Step::BatchCancel(_)
        | Step::BatchList(_) => return Err(failure(OTHER)),
    };
    let result = runtime.block_on(dispatch(AzureResourceOperation::Files(Box::pin(call(
        &opened, &fake, operation,
    )))));
    match result {
        Ok(AzureResourceResult::Files(reply)) => Ok(value(reply)),
        Ok(_) => Err(failure(OTHER)),
        Err(AzureResourceError::Files(AzureFilesError::Files { source })) => {
            Err(leaf_failure(&source))
        }
        Err(error) => Err(failure(error)),
    }
}

fn opened(fixture: &Fixture) -> Result<Opened, Box<HelperError>> {
    let key = SecretString::from(fixture.api_key.clone());
    let base_url = Url::parse(&fixture.api_base).map_err(failure)?;
    let client = SelectedClient::new(
        FilesClient {
            api_key: key.clone(),
            base_url,
            organization: None,
            timeout: DEFAULT_TIMEOUT,
            max_retries: 0,
        },
        &FileAuth::ApiKey(key),
        Some(fixture.api_version.clone()),
        ClientKind::AzureOpenAi,
    )
    .map_err(failure)?;
    Ok(Opened {
        client,
        is_async: false,
    })
}

fn create() -> FileCreate {
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

fn value(reply: FileReply) -> Value {
    match reply {
        FileReply::Object(object) => json!({"id": object.id, "bytes": object.bytes}),
        FileReply::Content(content) => {
            json!({"status": content.status, "length": content.body.len()})
        }
        FileReply::Page(page) => json!({"count": page.data.len()}),
        FileReply::Deleted(deleted) => json!({"deleted": deleted.deleted}),
    }
}

fn leaf_failure(source: &FilesError) -> Box<HelperError> {
    let FilesError::Status(status) = source else {
        return failure(source);
    };
    with_status(failure(status.message.clone()), status.status)
}
