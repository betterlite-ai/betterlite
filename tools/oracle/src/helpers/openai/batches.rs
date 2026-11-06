use std::sync::{Arc, PoisonError};

use betterlite_openai::batches::error::{BatchesError, SdkError};
use betterlite_openai::batches::records::{BatchCreate, BatchObject, BatchQuery};
use betterlite_openai::batches::sdk::{ClientKind, ClientSettings, Connection, Timeout};
use betterlite_openai::batches::{BatchOperation, BatchesApi};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{HelperError, HelperResult};

use self::fixture::{Constructor, Log, Sdk};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    api_key: Option<String>,
    api_base: Option<String>,
    timeout: Option<f64>,
    max_retries: Option<i64>,
    organization: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Failure {
    status: u16,
    #[serde(rename = "type")]
    error_type: Option<String>,
    param: Option<String>,
    code: Option<String>,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Answer {
    Value(Value),
    Failure(Failure),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Listing {
    after: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Request {
    Create(BatchCreate),
    Retrieve(BatchQuery),
    Cancel(BatchQuery),
    List(Listing),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    is_async: bool,
    supplied: Option<ClientKind>,
    #[serde(default)]
    unbuildable: bool,
    settings: Settings,
    request: Request,
    answer: Answer,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Reply {
    Batch(Box<BatchObject>),
    Page(Value),
}

#[derive(Serialize)]
struct Output {
    result: Option<Reply>,
    raised: Option<Failure>,
    calls: Vec<Value>,
    built: Vec<Value>,
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn settings(fields: &Settings) -> ClientSettings {
    ClientSettings {
        api_key: fields.api_key.clone().map(SecretString::from),
        api_base: fields.api_base.clone(),
        timeout: fields.timeout.map(Timeout::Seconds),
        max_retries: fields.max_retries,
        organization: fields.organization.clone(),
    }
}

async fn dispatch(
    api: &BatchesApi<Constructor>,
    request: &Request,
    connection: Connection<Sdk>,
) -> Result<Reply, BatchesError> {
    Ok(match request {
        Request::Create(create) => Reply::Batch(Box::new(
            api.batch(connection, BatchOperation::Create(create))
                .await?,
        )),
        Request::Retrieve(query) => Reply::Batch(Box::new(
            api.batch(connection, BatchOperation::Retrieve(query))
                .await?,
        )),
        Request::Cancel(query) => Reply::Batch(Box::new(
            api.batch(connection, BatchOperation::Cancel(query)).await?,
        )),
        Request::List(listing) => Reply::Page(
            api.list_batches(connection, listing.after.as_deref(), listing.limit)
                .await?,
        ),
    })
}

fn raised(error: BatchesError) -> Result<Failure, Box<HelperError>> {
    match error {
        BatchesError::Sdk {
            source:
                SdkError::Status {
                    status,
                    error_type,
                    param,
                    code,
                    message,
                },
        } => Ok(Failure {
            status,
            error_type,
            param,
            code,
            message,
        }),
        BatchesError::Invalid { .. } => Err(helper_error("ValidationError", error.to_string())),
        BatchesError::NotInitialized | BatchesError::NotAsync | BatchesError::NotSync => {
            Err(helper_error("ValueError", error.to_string()))
        }
        BatchesError::Sdk { .. } => Err(helper_error("SdkError", error.to_string())),
    }
}

async fn drive(call: &Call) -> HelperResult {
    let calls = Log::default();
    let built = Log::default();
    let api = BatchesApi {
        constructor: Constructor {
            unbuildable: call.unbuildable,
            calls: Arc::clone(&calls),
            built: Arc::clone(&built),
            answer: call.answer.clone(),
        },
    };
    let connection = Connection {
        is_async: call.is_async,
        settings: settings(&call.settings),
        client: call.supplied.map(|kind| Sdk {
            kind,
            calls: Arc::clone(&calls),
            answer: call.answer.clone(),
        }),
    };
    let (result, failure) = match dispatch(&api, &call.request, connection).await {
        Ok(reply) => (Some(reply), None),
        Err(error) => (None, Some(raised(error)?)),
    };
    let output = Output {
        result,
        raised: failure,
        calls: calls.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        built: built.lock().unwrap_or_else(PoisonError::into_inner).clone(),
    };
    serde_json::to_value(output).map_err(|error| helper_error("InvalidOutput", error.to_string()))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| helper_error("RuntimeError", error.to_string()))?
            .block_on(drive(input))
    }
}

mod fixture;
mod tests;
