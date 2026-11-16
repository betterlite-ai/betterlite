use std::time::Duration;

use betterlite_openai::batches::sdk::{ClientSettings, Connection as BatchConnection};
use betterlite_openai::files::records::{CreatePurpose, FileCreate, FileUpload};
use betterlite_openai::files::{FilesApi, FilesClient};
use betterlite_openai::fine_tuning::client::ClientSettings as JobSettings;
use betterlite_openai::fine_tuning::client::Connection as JobConnection;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_resource_dispatch::batches::{
    Call as BatchCall, Dispatch as BatchDispatch, Outcome as BatchOutcome,
};
use betterlite_provider::openai_resource_dispatch::files::{
    Call as FilesCall, Dispatch as FilesDispatch, Outcome as FilesOutcome,
};
use betterlite_provider::openai_resource_dispatch::fine_tuning::{
    Call as JobCall, Dispatch as JobDispatch, Outcome as JobOutcome,
};
use betterlite_provider::openai_resource_dispatch::{
    OpenAiResourceOperation, OpenAiResourceResult, dispatch,
};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::json;
use uuid::Uuid;

use super::fixtures::{Fixture, NoClient};
use super::{BatchesCreate, FilesCreate, FineTuningCreate, OTHER_OUTCOME, failure, leaf_failure};

fn file_create(case: &FilesCreate) -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: Some(case.filename.clone()),
            content_type: None,
            data: case.content.clone().into_bytes(),
        },
        purpose: CreatePurpose::Batch,
        expires_after: None,
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    }
}

fn files_client(case: &FilesCreate) -> Result<FilesClient, Box<HelperError>> {
    Ok(FilesClient {
        api_key: SecretString::from("sk-fixture"),
        base_url: case
            .api_base
            .parse()
            .map_err(|error| failure("InvalidInput", error))?,
        organization: None,
        timeout: Duration::from_secs(600),
        max_retries: 0,
    })
}

fn files_api(case: &FilesCreate) -> FilesApi<Fixture> {
    FilesApi {
        transport: Fixture {
            status: case.status,
            body: case.reply.to_string().into_bytes(),
        },
        boundaries: || Uuid::from_u128(1),
    }
}

pub(super) async fn files_create(case: &FilesCreate) -> HelperResult {
    let client = files_client(case)?;
    let api = files_api(case);
    let data = file_create(case);
    let call = FilesDispatch {
        api: &api,
        client: &client,
        call: FilesCall::Create(&data),
    };
    let operation = OpenAiResourceOperation::Files(Box::pin(call.run()));
    let outcome = dispatch(operation).await.map_err(leaf_failure)?;
    let OpenAiResourceResult::Files(FilesOutcome::Object(file)) = outcome else {
        return Err(failure("Variant", OTHER_OUTCOME));
    };
    Ok(json!({ "result": file }))
}

pub(super) async fn batches_create(case: &BatchesCreate) -> HelperResult {
    let api = betterlite_openai::batches::BatchesApi {
        constructor: NoClient,
    };
    let connection = BatchConnection {
        is_async: false,
        settings: ClientSettings::default(),
        client: Some(Fixture {
            status: case.status,
            body: case.reply.to_string().into_bytes(),
        }),
    };
    let call = BatchDispatch {
        api: &api,
        connection,
        call: BatchCall::Create(&case.create),
    };
    let operation = OpenAiResourceOperation::Batches(Box::pin(call.run()));
    let outcome = dispatch(operation).await.map_err(leaf_failure)?;
    let OpenAiResourceResult::Batches(BatchOutcome::Batch(batch)) = outcome else {
        return Err(failure("Variant", OTHER_OUTCOME));
    };
    Ok(json!({ "result": {
        "id": batch.id,
        "object": batch.object,
        "endpoint": batch.endpoint,
        "completion_window": batch.completion_window,
        "status": batch.status,
        "input_file_id": batch.input_file_id,
        "metadata": batch.metadata,
    }}))
}

pub(super) async fn fine_tuning_create(case: &FineTuningCreate) -> HelperResult {
    let connection = JobConnection {
        settings: JobSettings::default(),
        litellm_params: None,
        client: Some(Box::new(Fixture {
            status: case.status,
            body: case.reply.to_string().into_bytes(),
        })),
        is_async: false,
        builder: &NoClient,
    };
    let operation = OpenAiResourceOperation::FineTuning(Box::new(JobDispatch {
        connection,
        call: JobCall::Create(&case.create),
    }));
    let outcome = dispatch(operation).await.map_err(leaf_failure)?;
    let OpenAiResourceResult::FineTuning(JobOutcome::Job(job)) = outcome else {
        return Err(failure("Variant", OTHER_OUTCOME));
    };
    Ok(json!({ "result": {
        "id": job.id,
        "object": job.object,
        "status": job.status,
        "model": job.model,
        "training_file": job.training_file,
        "organization_id": job.organization_id,
        "seed": job.seed,
    }}))
}
