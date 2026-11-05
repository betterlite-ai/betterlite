use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_openai::batches::BatchesApi;
use betterlite_openai::batches::error::{BatchesError, SdkError};
use betterlite_openai::batches::records::{BatchCreate, BatchObject, BatchQuery};
use betterlite_openai::batches::sdk::{
    BatchesSdk, ClientConstructor, ClientKind, ClientSettings, Connection, SdkArguments, Timeout,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::helpers::{HelperError, HelperResult};

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
    limit: Option<u32>,
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

type Log = Arc<Mutex<Vec<Value>>>;

#[derive(Serialize)]
struct Kwargs<'arguments> {
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<&'arguments str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<&'arguments str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout: Option<&'arguments Timeout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_retries: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    organization: Option<&'arguments str>,
}

#[derive(Serialize)]
struct Built<'arguments> {
    kind: ClientKind,
    kwargs: Kwargs<'arguments>,
}

fn record(log: &Log, entry: Value) {
    log.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(entry);
}

struct Sdk {
    kind: ClientKind,
    calls: Log,
    answer: Answer,
}

impl Sdk {
    fn reply(&self, method: &str, kwargs: Value) -> Result<Value, SdkError> {
        let mut entry = Map::new();
        entry.insert(String::from("method"), Value::String(method.to_owned()));
        entry.insert(String::from("kwargs"), kwargs);
        record(&self.calls, Value::Object(entry));
        match &self.answer {
            Answer::Value(value) => Ok(value.clone()),
            Answer::Failure(failure) => Err(SdkError::Status {
                status: failure.status,
                error_type: failure.error_type.clone(),
                param: failure.param.clone(),
                code: failure.code.clone(),
                message: failure.message.clone(),
            }),
        }
    }

    fn encoded<Request>(&self, method: &str, request: &Request) -> Result<Vec<u8>, SdkError>
    where
        Request: Serialize,
    {
        let kwargs = serde_json::to_value(request).map_err(|source| SdkError::Encode { source })?;
        let value = self.reply(method, kwargs)?;
        serde_json::to_vec(&value).map_err(|source| SdkError::Encode { source })
    }
}

impl BatchesSdk for Sdk {
    type Page = Value;

    fn kind(&self) -> ClientKind {
        self.kind
    }

    fn create(
        &self,
        request: &BatchCreate,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.encoded("create", request))
    }

    fn retrieve(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.encoded("retrieve", request))
    }

    fn cancel(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.encoded("cancel", request))
    }

    fn list(
        &self,
        after: Option<&str>,
        limit: Option<u32>,
    ) -> impl Future<Output = Result<Value, SdkError>> + Send {
        let mut kwargs = Map::new();
        kwargs.insert(
            String::from("after"),
            after.map_or(Value::Null, |value| Value::String(value.to_owned())),
        );
        kwargs.insert(
            String::from("limit"),
            limit.map_or(Value::Null, |value| {
                Value::Number(serde_json::Number::from(value))
            }),
        );
        ready(self.reply("list", Value::Object(kwargs)))
    }
}

struct Constructor {
    unbuildable: bool,
    calls: Log,
    built: Log,
    answer: Answer,
}

impl ClientConstructor for Constructor {
    type Client = Sdk;

    fn construct(&self, arguments: &SdkArguments, kind: ClientKind) -> Option<Sdk> {
        let built = Built {
            kind,
            kwargs: Kwargs {
                api_key: arguments.api_key.as_ref().map(ExposeSecret::expose_secret),
                base_url: arguments.base_url.as_deref(),
                timeout: arguments.timeout.as_ref(),
                max_retries: arguments.max_retries,
                organization: arguments.organization.as_deref(),
            },
        };
        record(&self.built, serde_json::to_value(built).unwrap_or_default());
        (!self.unbuildable).then(|| Sdk {
            kind,
            calls: Arc::clone(&self.calls),
            answer: self.answer.clone(),
        })
    }
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
        Request::Create(create) => {
            Reply::Batch(Box::new(api.create_batch(connection, create).await?))
        }
        Request::Retrieve(query) => {
            Reply::Batch(Box::new(api.retrieve_batch(connection, query).await?))
        }
        Request::Cancel(query) => {
            Reply::Batch(Box::new(api.cancel_batch(connection, query).await?))
        }
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

mod tests;
