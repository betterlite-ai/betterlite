use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_openai::batches::error::SdkError;
use betterlite_openai::batches::records::{BatchCreate, BatchQuery};
use betterlite_openai::batches::sdk::{
    BatchesSdk, ClientConstructor, ClientKind, SdkArguments, Timeout,
};
use secrecy::ExposeSecret;
use serde::Serialize;
use serde_json::{Map, Value};

use super::Answer;

pub(super) type Log = Arc<Mutex<Vec<Value>>>;

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

pub(super) fn record(log: &Log, entry: Value) {
    log.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(entry);
}

pub(super) struct Sdk {
    pub(super) kind: ClientKind,
    pub(super) calls: Log,
    pub(super) answer: Answer,
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
        limit: Option<i64>,
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

pub(super) struct Constructor {
    pub(super) unbuildable: bool,
    pub(super) calls: Log,
    pub(super) built: Log,
    pub(super) answer: Answer,
}

impl ClientConstructor for Constructor {
    type Client = Sdk;

    fn construct(
        &self,
        arguments: &SdkArguments,
        kind: ClientKind,
    ) -> Result<Option<Sdk>, SdkError> {
        if self.unbuildable {
            return Ok(None);
        }
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
        Ok(Some(Sdk {
            kind,
            calls: Arc::clone(&self.calls),
            answer: self.answer.clone(),
        }))
    }
}
