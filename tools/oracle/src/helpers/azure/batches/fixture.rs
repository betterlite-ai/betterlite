use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::batches::client::ClientMaterializer;
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{ClientIdentity, ClientKind, ClientOutcome};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_http::client_kind::ClientKind as SdkKind;
use betterlite_openai::batches::error::SdkError;
use betterlite_openai::batches::records::{BatchCreate, BatchPage, BatchQuery};
use betterlite_openai::batches::sdk::BatchesSdk;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use super::steps::BuilderMode;

pub(crate) type Log = Arc<Mutex<Vec<Value>>>;

// What the case says the SDK answers; a failure carries the SDK's error body under "error"
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Planned {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

impl Planned {
    fn failure(&self) -> SdkError {
        let field = |name: &str| {
            self.body
                .get("error")
                .and_then(|error| error.get(name))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        SdkError::Status {
            status: self.status,
            error_type: field("type"),
            param: field("param"),
            code: field("code"),
            message: field("message").unwrap_or_default(),
        }
    }
}

pub(crate) struct RecordedSdk {
    kind: SdkKind,
    reply: Option<Planned>,
    log: Log,
}

impl RecordedSdk {
    fn answer(&self, entry: Value) -> Result<Vec<u8>, SdkError> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
        match &self.reply {
            Some(planned) if planned.status >= 400 => Err(planned.failure()),
            Some(planned) => Ok(planned.body.to_string().into_bytes()),
            None => Ok(b"{}".to_vec()),
        }
    }
}

impl BatchesSdk for RecordedSdk {
    type Page = BatchPage;

    fn kind(&self) -> SdkKind {
        self.kind
    }

    fn create(
        &self,
        request: &BatchCreate,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.answer(json!({ "create": request })))
    }

    fn retrieve(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.answer(json!({ "retrieve": request })))
    }

    fn cancel(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.answer(json!({ "cancel": request })))
    }

    fn list(
        &self,
        after: Option<&str>,
        limit: Option<i64>,
    ) -> impl Future<Output = Result<BatchPage, SdkError>> + Send {
        let answer = self.answer(json!({ "list": { "after": after, "limit": limit } }));
        ready(answer.and_then(|bytes| {
            serde_json::from_slice(&bytes).map_err(|source| SdkError::Body { source })
        }))
    }
}

pub(crate) struct Materializer {
    pub(crate) mode: BuilderMode,
    pub(crate) reply: Option<Planned>,
    pub(crate) log: Log,
}

impl ClientMaterializer for Materializer {
    type Client = RecordedSdk;

    fn materialize(&self, outcome: &ClientOutcome<'_>) -> Result<Option<RecordedSdk>, SdkError> {
        if self.mode == BuilderMode::None {
            return Ok(None);
        }
        Ok(Some(RecordedSdk {
            kind: sdk_kind(outcome.kind),
            reply: self.reply.clone(),
            log: Arc::clone(&self.log),
        }))
    }
}

const fn sdk_kind(kind: ClientKind) -> SdkKind {
    match kind {
        ClientKind::AsyncAzureOpenAi | ClientKind::AsyncOpenAi => SdkKind::Asynchronous,
        ClientKind::AzureOpenAi | ClientKind::OpenAi | ClientKind::Other => SdkKind::Sync,
    }
}

pub(crate) struct Sources;

fn source_failure() -> SourceFailure {
    SourceFailure {
        class: "Unavailable".to_owned(),
        status: None,
        message: "the fixture has no credential sources".to_owned(),
    }
}

impl CredentialSources for Sources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(source_failure())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(source_failure())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(source_failure())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(source_failure())
    }
}

#[derive(Default)]
pub(crate) struct Cache;

impl ClientCache for Cache {
    fn get(&self, _identity: &ClientIdentity) -> Option<ClientKind> {
        None
    }

    fn set(&mut self, _identity: ClientIdentity, _kind: ClientKind, _owned: bool) {}
}
