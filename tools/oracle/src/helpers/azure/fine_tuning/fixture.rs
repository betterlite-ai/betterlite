use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{ClientIdentity, ClientKind, ClientOutcome};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_openai::fine_tuning::client::{BuildError, JobsApi, SdkError};
use betterlite_openai::fine_tuning::job::FineTuneCreate;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use super::steps::BuilderMode;

pub(crate) type Log = Arc<Mutex<Vec<Value>>>;
pub(crate) type Reply = Result<Vec<u8>, SdkError>;

#[derive(Clone, Deserialize)]
pub(crate) struct Planned {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

pub(crate) fn planned_reply(planned: &Option<Planned>) -> Reply {
    let Some(Planned { status, body }) = planned else {
        return Ok(b"{}".to_vec());
    };
    if *status < 400 {
        return Ok(body.to_string().into_bytes());
    }
    let text = |name: &str| {
        let field = body.get("error").and_then(|error| error.get(name));
        field.and_then(Value::as_str).map(str::to_owned)
    };
    Err(SdkError {
        status: *status,
        error_type: text("type"),
        param: text("param"),
        code: text("code"),
        message: text("message").unwrap_or_default(),
    })
}

pub(crate) struct Recorded {
    is_async: bool,
    pub(crate) reply: Reply,
    pub(crate) log: Log,
}

impl Recorded {
    pub(crate) fn new(is_async: bool, reply: Reply, log: Log) -> Self {
        Self {
            is_async,
            reply,
            log,
        }
    }

    fn answer(&self, call: Value) -> Reply {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.reply.clone()
    }
}

// the SDK merges create_fine_tuning_job_data.extra_body into the request body, so a recorded create is the flattened body
fn wire_body(data: &FineTuneCreate) -> Value {
    let mut body = json!(data);
    if let Some(map) = body.as_object_mut() {
        let extra = map
            .remove("extra_body")
            .and_then(|value| value.as_object().cloned());
        if let Some(extra) = extra {
            map.extend(extra);
        }
    }
    body
}

impl JobsApi for Recorded {
    fn is_async(&self) -> bool {
        self.is_async
    }

    fn create(&self, data: &FineTuneCreate) -> Reply {
        self.answer(json!({ "create": wire_body(data) }))
    }

    fn list(&self, after: Option<&str>, limit: Option<i64>) -> Reply {
        self.answer(json!({ "list": { "after": after, "limit": limit } }))
    }

    fn retrieve(&self, job_id: &str) -> Reply {
        self.answer(json!({ "retrieve": { "fine_tuning_job_id": job_id } }))
    }

    fn cancel(&self, job_id: &str) -> Reply {
        self.answer(json!({ "cancel": { "fine_tuning_job_id": job_id } }))
    }
}

pub(crate) struct Materializer {
    pub(crate) mode: BuilderMode,
    pub(crate) reply: Reply,
    pub(crate) calls: Log,
}

impl betterlite_azure::fine_tuning::client::ClientMaterializer for Materializer {
    fn materialize(
        &self,
        outcome: &ClientOutcome<'_>,
    ) -> Result<Option<Box<dyn JobsApi>>, BuildError> {
        if self.mode == BuilderMode::None {
            return Ok(None);
        }
        let built_async = self.mode == BuilderMode::Default
            && matches!(
                outcome.kind,
                ClientKind::AsyncAzureOpenAi | ClientKind::AsyncOpenAi
            );
        Ok(Some(Box::new(Recorded::new(
            built_async,
            self.reply.clone(),
            Arc::clone(&self.calls),
        ))))
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
