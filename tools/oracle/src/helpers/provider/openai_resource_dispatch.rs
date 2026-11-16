use betterlite_openai::batches::error::{BatchesError, SdkError as BatchSdkError};
use betterlite_openai::batches::records::BatchCreate;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::fine_tuning::{error::FineTuneError, job::FineTuneCreate};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::vector_stores::store::CreateParams;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::openai_resource_dispatch::OpenAiResourceError;
use serde::Deserialize;
use serde_json::Value;
use tokio::runtime::Builder;

mod fixtures;
mod sdk_handlers;
mod transform_handlers;

pub(crate) struct Op;

// Reached only if an operation and the result variant it reads back ever disagree
pub(super) const OTHER_OUTCOME: &str = "the dispatch returned another outcome";

// The oracle calls one operation per input; `case` names it the way the Python adapter does
#[derive(Debug, Deserialize)]
#[serde(tag = "case", rename_all = "snake_case")]
enum Input {
    FilesCreate(FilesCreate),
    BatchesCreate(BatchesCreate),
    FineTuningCreate(FineTuningCreate),
    ContainersList(ContainersList),
    EvalsGet(EvalsGet),
    VectorStoresCreate(VectorStoresCreate),
}

#[derive(Debug, Deserialize)]
struct FilesCreate {
    api_base: String,
    status: u16,
    filename: String,
    content: String,
    reply: Value,
}

#[derive(Debug, Deserialize)]
struct BatchesCreate {
    status: u16,
    create: BatchCreate,
    reply: Value,
}

#[derive(Debug, Deserialize)]
struct FineTuningCreate {
    status: u16,
    create: FineTuneCreate,
    reply: Value,
}

#[derive(Debug, Deserialize)]
struct ContainersList {
    api_base: String,
    after: Option<String>,
    limit: Option<i64>,
    order: Option<String>,
    extra_query: Option<Object>,
}

#[derive(Debug, Deserialize)]
struct EvalsGet {
    api_base: String,
    eval_id: String,
}

#[derive(Debug, Deserialize)]
struct VectorStoresCreate {
    api_base: String,
    params: CreateParams,
}

fn failure<M: std::fmt::Display>(class: &str, message: M) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

// LiteLLM's SDK raises the upstream status; every other leaf failure carries no status
fn leaf_failure(error: OpenAiResourceError) -> Box<HelperError> {
    let status = if let OpenAiResourceError::Files(FilesError::Status(raised)) = &error {
        Some(raised.status)
    } else if let OpenAiResourceError::Batches(BatchesError::Sdk {
        source: BatchSdkError::Status { status, .. },
    }) = &error
    {
        Some(*status)
    } else if let OpenAiResourceError::FineTuning(FineTuneError::Sdk { source }) = &error {
        Some(source.status)
    } else {
        None
    };
    Box::new(HelperError {
        class: "LeafFailure".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        runtime.block_on(dispatch_input(input))
    }
}

async fn dispatch_input(input: &Input) -> HelperResult {
    match input {
        Input::FilesCreate(case) => sdk_handlers::files_create(case).await,
        Input::BatchesCreate(case) => sdk_handlers::batches_create(case).await,
        Input::FineTuningCreate(case) => sdk_handlers::fine_tuning_create(case).await,
        Input::ContainersList(case) => transform_handlers::containers_list(case).await,
        Input::EvalsGet(case) => transform_handlers::evals_get(case).await,
        Input::VectorStoresCreate(case) => transform_handlers::vector_stores_create(case).await,
    }
}
