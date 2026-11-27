use std::collections::{BTreeMap, VecDeque};
use std::fmt::{Debug, Write as _};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::TokenRequest;
use tokio::runtime::Builder;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_input_accounting::error::{
    BatchHttpError, BatchInputAccountingError, BatchManagedFileError,
};
use crate::api::batch_input_accounting::input::BatchInputAccountingInput;
use crate::api::batch_input_accounting::ports::{
    BatchAccessCheck, BatchAccountingCalls, BatchFetchParams, BatchFileFetch, BatchFileRequest,
    BatchFileSource, BatchKeyGrant, BatchModelAccess, BatchTeamLookup,
};
use crate::api::batch_input_accounting::result::{BatchInputAccountingResult, BatchInputRowUsage};
use crate::api::batch_input_accounting::service::BatchInputAccounting;
use crate::api::batch_output_usage::tokens::BatchTokenCounter;
use crate::api::request_destination::RequestData;

fn locked<Value>(mutex: &Mutex<Value>) -> MutexGuard<'_, Value> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn block_on<Output, Fut>(future: Fut) -> Output
where
    Fut: Future<Output = Output>,
{
    Builder::new_current_thread()
        .build()
        .expect("the test runtime builds")
        .block_on(future)
}

pub(super) fn input_wire(file_id: &str, caller: Option<&str>, descriptors: &str) -> String {
    let mut wire = format!(r#"{{"file_id": "{file_id}", "descriptors": {descriptors}"#);
    if let Some(principal) = caller {
        write!(wire, r#", "caller": {principal}"#).expect("writing to a string cannot fail");
    }
    wire.push('}');
    wire
}

// A unified managed file id for the given comma-separated upload target names
pub(super) fn managed_file_id(target_names: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!(
        "litellm_proxy:application/octet-stream;unified_id,file-1;target_model_names,{target_names}"
    ))
}

pub(super) const fn row_usage(
    total_tokens: i64,
    output_tokens: i64,
    request_count: i64,
) -> BatchInputRowUsage {
    BatchInputRowUsage {
        total_tokens,
        output_tokens,
        request_count,
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted calls port failed")]
pub(super) struct CallsFailure;

#[derive(Debug, thiserror::Error)]
#[error("the scripted files port is offline")]
pub(super) struct FilesFailure;

#[derive(Debug, thiserror::Error)]
#[error("the scripted access port is offline")]
pub(super) struct AccessPortError;

pub(super) type FakeError = BatchInputAccountingError<CallsFailure, FilesFailure, AccessPortError>;

// B1044's answers are scripted per row; a missing answer counts zero and a queued None fails the call
#[derive(Default)]
pub(super) struct ScriptedCalls {
    pub(super) floor: i64,
    estimates: Mutex<VecDeque<Option<i64>>>,
    seen_min: Mutex<Vec<Option<i64>>>,
    resolves: Mutex<Vec<(String, String)>>,
}

impl ScriptedCalls {
    pub(super) fn script(&self, tokens: &[i64]) {
        locked(&self.estimates).extend(tokens.iter().copied().map(Some));
    }

    pub(super) fn fail_next(&self) {
        locked(&self.estimates).push_back(None);
    }

    pub(super) fn seen_min(&self) -> Vec<Option<i64>> {
        locked(&self.seen_min).clone()
    }

    pub(super) fn resolves(&self) -> Vec<(String, String)> {
        locked(&self.resolves).clone()
    }
}

impl BatchAccountingCalls for ScriptedCalls {
    type Error = CallsFailure;

    fn output_floor(&self, min_configured_otpm_limit: Option<i64>) -> i64 {
        locked(&self.seen_min).push(min_configured_otpm_limit);
        self.floor
    }

    fn estimate_output_tokens(
        &self,
        _entry: &RequestData,
        min_configured_otpm_limit: Option<i64>,
    ) -> Result<i64, CallsFailure> {
        locked(&self.seen_min).push(min_configured_otpm_limit);
        let answer = locked(&self.estimates).pop_front();
        match answer {
            Some(Some(tokens)) => Ok(tokens),
            Some(None) => Err(CallsFailure),
            None => Ok(0),
        }
    }

    fn resolve_fetch_params(
        &self,
        file_id: &str,
        custom_llm_provider: &str,
        _data: &RequestData,
    ) -> Result<BatchFetchParams, CallsFailure> {
        locked(&self.resolves).push((file_id.to_owned(), custom_llm_provider.to_owned()));
        let mut params = JsonObject::new();
        params.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(custom_llm_provider.to_owned()),
        );
        Ok(BatchFetchParams {
            file_id: format!("{file_id}-resolved"),
            params,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RecordedFetch {
    Managed {
        file_id: String,
        caller: bool,
    },
    Provider {
        file_id: String,
        params: JsonObject,
        caller: bool,
    },
}

enum FilesStep {
    Content(Vec<u8>),
    NotBytes(String),
    Managed(BatchManagedFileError),
    Http(BatchHttpError),
    Fail,
}

#[derive(Default)]
pub(super) struct ScriptedFiles {
    steps: Mutex<VecDeque<FilesStep>>,
    recorded: Mutex<Vec<RecordedFetch>>,
}

impl ScriptedFiles {
    pub(super) fn content(&self, bytes: &[u8]) {
        locked(&self.steps).push_back(FilesStep::Content(bytes.to_vec()));
    }

    pub(super) fn not_bytes(&self, kind: &str) {
        locked(&self.steps).push_back(FilesStep::NotBytes(kind.to_owned()));
    }

    pub(super) fn managed_unavailable(&self, reason: BatchManagedFileError) {
        locked(&self.steps).push_back(FilesStep::Managed(reason));
    }

    pub(super) fn http(&self, http: BatchHttpError) {
        locked(&self.steps).push_back(FilesStep::Http(http));
    }

    pub(super) fn fail(&self) {
        locked(&self.steps).push_back(FilesStep::Fail);
    }

    pub(super) fn recorded(&self) -> Vec<RecordedFetch> {
        locked(&self.recorded).clone()
    }
}

#[async_trait]
impl BatchFileSource for ScriptedFiles {
    type Error = FilesFailure;

    async fn fetch(&self, request: BatchFileRequest<'_>) -> Result<BatchFileFetch, FilesFailure> {
        let recorded = match request {
            BatchFileRequest::Managed { file_id, .. } => RecordedFetch::Managed {
                file_id: file_id.to_owned(),
                caller: true,
            },
            BatchFileRequest::Provider {
                file_id,
                params,
                caller,
            } => RecordedFetch::Provider {
                file_id: file_id.to_owned(),
                params: params.clone(),
                caller: caller.is_some(),
            },
        };
        locked(&self.recorded).push(recorded);
        let step = locked(&self.steps).pop_front();
        match step {
            Some(FilesStep::Content(bytes)) => Ok(BatchFileFetch::Content(bytes)),
            Some(FilesStep::NotBytes(kind)) => Ok(BatchFileFetch::NotBytes { kind }),
            Some(FilesStep::Managed(reason)) => Ok(BatchFileFetch::ManagedUnavailable(reason)),
            Some(FilesStep::Http(http)) => Ok(BatchFileFetch::Http(http)),
            Some(FilesStep::Fail) => Err(FilesFailure),
            None => Ok(BatchFileFetch::Content(Vec::new())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AccessMethod {
    Team,
    TeamAccess,
    Grant,
    Member,
    Key,
}

impl AccessMethod {
    const fn label(self) -> &'static str {
        match self {
            Self::Team => "team",
            Self::TeamAccess => "team_access",
            Self::Grant => "grant",
            Self::Member => "member",
            Self::Key => "key",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Team;

type Checked = Result<BatchAccessCheck, AccessPortError>;
type Granted = Result<BatchKeyGrant, AccessPortError>;

#[derive(Default)]
pub(super) struct ScriptedAccess {
    pub(super) database: bool,
    pub(super) team_checks: BTreeMap<String, BatchAccessCheck>,
    pub(super) grants: BTreeMap<String, BatchKeyGrant>,
    pub(super) member_checks: BTreeMap<String, BatchAccessCheck>,
    pub(super) key_checks: BTreeMap<String, BatchAccessCheck>,
    pub(super) failure: Option<AccessMethod>,
    teams: Mutex<VecDeque<BatchTeamLookup<Team>>>,
    asked: Mutex<Vec<String>>,
}

impl ScriptedAccess {
    pub(super) fn script_team(&self, results: Vec<BatchTeamLookup<Team>>) {
        locked(&self.teams).extend(results);
    }

    pub(super) fn asked(&self) -> Vec<String> {
        locked(&self.asked).clone()
    }

    fn note(&self, label: String) {
        locked(&self.asked).push(label);
    }

    fn begin(&self, method: AccessMethod, model: &str) -> Result<(), AccessPortError> {
        self.note(format!("{}:{model}", method.label()));
        if self.failure == Some(method) {
            return Err(AccessPortError);
        }
        Ok(())
    }

    fn verdict(
        &self,
        method: AccessMethod,
        model: &str,
        checks: &BTreeMap<String, BatchAccessCheck>,
    ) -> Checked {
        self.begin(method, model)?;
        Ok(checks
            .get(model)
            .cloned()
            .unwrap_or(BatchAccessCheck::Allowed))
    }

    fn team_verdict(&self, model: &str) -> Checked {
        self.verdict(AccessMethod::TeamAccess, model, &self.team_checks)
    }

    fn member_verdict(&self, model: &str) -> Checked {
        self.verdict(AccessMethod::Member, model, &self.member_checks)
    }

    fn key_verdict(&self, model: &str) -> Checked {
        self.verdict(AccessMethod::Key, model, &self.key_checks)
    }

    fn grant_verdict(&self, model: &str) -> Granted {
        self.begin(AccessMethod::Grant, model)?;
        Ok(self
            .grants
            .get(model)
            .cloned()
            .unwrap_or(BatchKeyGrant::Denied))
    }
}

#[async_trait]
impl BatchModelAccess for ScriptedAccess {
    type Team = Team;
    type Error = AccessPortError;

    fn database_available(&self) -> bool {
        self.database
    }

    async fn team(&self, team_id: &str) -> Result<BatchTeamLookup<Team>, AccessPortError> {
        self.note(format!("team:{team_id}"));
        if self.failure == Some(AccessMethod::Team) {
            return Err(AccessPortError);
        }
        locked(&self.teams).pop_front().ok_or(AccessPortError)
    }

    async fn team_can_access(&self, model: &str, _key: &UserAPIKeyAuth, _team: &Team) -> Checked {
        self.team_verdict(model)
    }

    async fn key_group_grants(&self, model: &str, _key: &UserAPIKeyAuth, _team: &Team) -> Granted {
        self.grant_verdict(model)
    }

    async fn member_can_access(&self, model: &str, _key: &UserAPIKeyAuth, _team: &Team) -> Checked {
        self.member_verdict(model)
    }

    async fn key_can_call(&self, model: &str, _key: &UserAPIKeyAuth) -> Checked {
        self.key_verdict(model)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CountedCall {
    Text { model: String, text: String },
    Messages { model: String, roles: Vec<String> },
    None { model: String },
}

impl CountedCall {
    fn of(request: &TokenRequest<'_>) -> Self {
        let model = request.model.to_owned();
        match (request.text, request.messages) {
            (Some(text), _) => Self::Text {
                model,
                text: text.joined().into_owned(),
            },
            (None, Some(messages)) => Self::Messages {
                model,
                roles: messages
                    .iter()
                    .map(|message| message.role.clone())
                    .collect(),
            },
            (None, None) => Self::None { model },
        }
    }

    fn shape(&self) -> (String, String, String) {
        match self {
            Self::Text { model, text } => ("text".to_owned(), model.clone(), text.clone()),
            Self::Messages { model, roles } => {
                ("messages".to_owned(), model.clone(), roles.join(","))
            }
            Self::None { model } => ("none".to_owned(), model.clone(), String::new()),
        }
    }
}

#[derive(Default)]
pub(super) struct ScriptedCounter {
    answers: Mutex<VecDeque<Option<usize>>>,
    calls: Mutex<Vec<CountedCall>>,
}

impl ScriptedCounter {
    pub(super) fn script(&self, tokens: &[usize]) {
        locked(&self.answers).extend(tokens.iter().copied().map(Some));
    }

    pub(super) fn script_answers(&self, answers: &[Option<usize>]) {
        locked(&self.answers).extend(answers.iter().copied());
    }

    pub(super) fn calls(&self) -> Vec<CountedCall> {
        locked(&self.calls).clone()
    }

    pub(super) fn shapes(&self) -> Vec<(String, String, String)> {
        locked(&self.calls).iter().map(CountedCall::shape).collect()
    }
}

#[async_trait]
impl BatchTokenCounter for ScriptedCounter {
    async fn count_tokens(&self, request: &TokenRequest<'_>) -> Result<usize, CountError> {
        let call = CountedCall::of(request);
        let answer = locked(&self.answers).pop_front();
        locked(&self.calls).push(call);
        match answer {
            Some(Some(tokens)) => Ok(tokens),
            Some(None) => Err(CountError::MissingInput),
            None => Ok(0),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured {
    entries: Arc<Mutex<Vec<(Level, String)>>>,
}

impl Captured {
    pub(super) fn lines(&self, level: Level) -> Vec<String> {
        locked(&self.entries)
            .iter()
            .filter(|(seen, _line)| *seen == level)
            .map(|(_seen, line)| line.clone())
            .collect()
    }

    pub(super) fn silence(&self) -> bool {
        locked(&self.entries).is_empty()
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        locked(&self.entries).push((*event.metadata().level(), message.0));
    }
}

#[derive(Default)]
pub(super) struct Harness {
    pub(super) calls: ScriptedCalls,
    pub(super) files: ScriptedFiles,
    pub(super) access: ScriptedAccess,
    pub(super) counter: ScriptedCounter,
    pub(super) logs: Captured,
    pub(super) limits: ApiLimits,
}

impl Harness {
    pub(super) fn new(content: &str) -> Self {
        let harness = Self::default();
        harness.files.content(content.as_bytes());
        harness
    }

    pub(super) fn run(&self, wire: &str) -> Result<BatchInputAccountingResult, FakeError> {
        let input: BatchInputAccountingInput =
            serde_json::from_str(wire).map_err(BatchInputAccountingError::Fields)?;
        let service = BatchInputAccounting::new(
            &self.calls,
            &self.files,
            &self.access,
            &self.counter,
            self.limits,
        );
        let guard = set_default(tracing_subscriber::registry().with(self.logs.clone()));
        let outcome = block_on(service.run(&input));
        drop(guard);
        outcome
    }
}
