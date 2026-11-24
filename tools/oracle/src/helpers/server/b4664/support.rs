use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::jwt_claims::TypedValue;
use betterlite_mcp::sso_assertions::assertion::{SsoIdentityAssertion, assertion_from_sso_login};
use betterlite_mcp::sso_assertions::read::{AssertionStoreUnavailable, SsoAssertionStore};
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::client_config::{
    SsoClientConfig, SsoClientSource, sso_client_config,
};
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::failure::{
    RefreshFailure, RefreshFailureKind,
};
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::refresher::AssertionAccess;
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::store::{
    CoordinatorFactory, RenewalCoordinator, RenewalUnavailable,
};
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::transport::{
    RefreshResponseBody, RefreshTransport,
};
use betterlite_server::mcp::outbound_credentials::{
    RefreshCoordinator, RefreshStep, TokenKey, WallClock,
};
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;

const HEADER_JSON: &[u8] = br#"{"alg":"none"}"#;
const READ_FAILURE: &str = "the scripted row read failed";
const WRITE_FAILURE: &str = "the scripted row write failed";

pub(crate) type TransportAnswer = Result<RefreshResponseBody, RefreshFailure>;

#[derive(Debug, Deserialize)]
pub(crate) struct ClientConfigInput {
    pub(crate) vars: BTreeMap<String, String>,
}

// The case names an id_token by the claims it carries, so both sides mint the same three-segment text.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AssertionSpec {
    claims: BTreeMap<String, Value>,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct BodySpec {
    #[serde(default)]
    claims: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    id_token: Option<Value>,
    #[serde(default)]
    refresh_token: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum OutcomeSpec {
    Ok {
        #[serde(default)]
        body: BodySpec,
    },
    Rejected {
        detail: String,
    },
    Unavailable {
        detail: String,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccessFailure {
    #[default]
    None,
    Read,
    Write,
}

pub(crate) fn runtime() -> Result<Runtime, Box<HelperError>> {
    let executor = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| value_error(error.to_string()))?;
    Ok(executor)
}

pub(crate) fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn unavailable(message: &str) -> AssertionStoreUnavailable {
    AssertionStoreUnavailable {
        cause: Box::new(io::Error::other(message.to_owned())),
    }
}

fn id_token_text(claims: &BTreeMap<String, Value>) -> Result<String, Box<HelperError>> {
    let payload = serde_json::to_string(claims).map_err(|error| value_error(error.to_string()))?;
    let header = URL_SAFE_NO_PAD.encode(HEADER_JSON);
    let body = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    Ok(format!("{header}.{body}."))
}

pub(crate) fn assertion_from_spec(
    spec: &AssertionSpec,
) -> Result<SsoIdentityAssertion, Box<HelperError>> {
    let token = TypedValue {
        json: Value::String(id_token_text(&spec.claims)?),
    };
    let refresh = spec.refresh_token.as_ref().map(|text| TypedValue {
        json: Value::String(text.clone()),
    });
    assertion_from_sso_login(Some(&token), refresh.as_ref())
        .ok_or_else(|| value_error("the case's assertion does not decode".to_owned()))
}

fn body_fields(spec: &BodySpec) -> Result<RefreshResponseBody, Box<HelperError>> {
    let mut body = BTreeMap::new();
    match (&spec.claims, &spec.id_token) {
        (Some(_claims), Some(_raw)) => {
            return Err(value_error(
                "a body names either claims or id_token, not both".to_owned(),
            ));
        }
        (Some(claims), None) => {
            body.insert("id_token".to_owned(), Value::String(id_token_text(claims)?));
        }
        (None, Some(raw)) => {
            body.insert("id_token".to_owned(), raw.clone());
        }
        (None, None) => {}
    }
    if let Some(rotated) = &spec.refresh_token {
        body.insert("refresh_token".to_owned(), rotated.clone());
    }
    Ok(body)
}

pub(crate) fn scripted(outcome: &OutcomeSpec) -> Result<TransportAnswer, Box<HelperError>> {
    match outcome {
        OutcomeSpec::Ok { body } => Ok(Ok(body_fields(body)?)),
        OutcomeSpec::Rejected { detail } => Ok(Err(RefreshFailure {
            kind: RefreshFailureKind::Rejected,
            detail: detail.clone(),
        })),
        OutcomeSpec::Unavailable { detail } => Ok(Err(RefreshFailure {
            kind: RefreshFailureKind::Unavailable,
            detail: detail.clone(),
        })),
    }
}

pub(crate) struct FixedClient {
    pub(crate) vars: BTreeMap<String, String>,
}

impl SsoClientSource for FixedClient {
    fn client(&self) -> Option<SsoClientConfig> {
        sso_client_config(&self.vars)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RecordedCall {
    pub(crate) url: String,
    pub(crate) form: BTreeMap<String, String>,
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Clone)]
pub(crate) struct ScriptedTransport {
    pub(crate) outcomes: Arc<Mutex<VecDeque<TransportAnswer>>>,
    pub(crate) calls: Arc<Mutex<Vec<RecordedCall>>>,
}

#[async_trait]
impl RefreshTransport for ScriptedTransport {
    async fn post(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> TransportAnswer {
        let call = RecordedCall {
            url: url.to_owned(),
            form: form.clone(),
            headers: headers.clone(),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| {
                Err(RefreshFailure {
                    kind: RefreshFailureKind::Unavailable,
                    detail: "the case scripted no outcome".to_owned(),
                })
            })
    }
}

pub(crate) struct MemoryAccess {
    pub(crate) rows: Mutex<BTreeMap<String, SsoIdentityAssertion>>,
    pub(crate) writes: Mutex<Vec<(String, SsoIdentityAssertion)>>,
    pub(crate) failure: AccessFailure,
}

#[async_trait]
impl AssertionAccess for MemoryAccess {
    async fn read(
        &self,
        user_id: &str,
    ) -> Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable> {
        if matches!(self.failure, AccessFailure::Read) {
            return Err(unavailable(READ_FAILURE));
        }
        Ok(self
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(user_id)
            .cloned())
    }

    async fn write(
        &self,
        user_id: &str,
        assertion: &SsoIdentityAssertion,
    ) -> Result<(), AssertionStoreUnavailable> {
        if matches!(self.failure, AccessFailure::Write) {
            return Err(unavailable(WRITE_FAILURE));
        }
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((user_id.to_owned(), assertion.clone()));
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(user_id.to_owned(), assertion.clone());
        Ok(())
    }
}

pub(crate) struct InnerStore {
    pub(crate) rows: Mutex<BTreeMap<String, SsoIdentityAssertion>>,
}

impl SsoAssertionStore for InnerStore {
    async fn fetch(
        &self,
        user_id: &str,
    ) -> Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable> {
        Ok(self
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(user_id)
            .cloned())
    }
}

pub(crate) struct FixedClock {
    now: DateTime<Utc>,
}

impl WallClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

pub(crate) fn clock(now_ms: i64) -> Result<Arc<dyn WallClock>, Box<HelperError>> {
    let now = DateTime::from_timestamp_millis(now_ms)
        .ok_or_else(|| value_error(format!("invalid now_ms: {now_ms}")))?;
    Ok(Arc::new(FixedClock { now }))
}

pub(crate) struct LoserCoordinator;

#[async_trait]
impl RefreshCoordinator for LoserCoordinator {
    type Output = ();
    type Error = RenewalUnavailable;

    async fn run(
        &self,
        _key: TokenKey,
        _refresh: RefreshStep<'_, (), RenewalUnavailable>,
        reread: RefreshStep<'_, (), RenewalUnavailable>,
    ) -> Result<(), RenewalUnavailable> {
        reread().await
    }
}

pub(crate) struct FixedFactory {
    pub(crate) loser: bool,
}

impl CoordinatorFactory for FixedFactory {
    fn coordinator(&self) -> Option<RenewalCoordinator> {
        if self.loser {
            return Some(Arc::new(LoserCoordinator));
        }
        None
    }
}

pub(crate) fn view(assertion: &SsoIdentityAssertion) -> Value {
    json!({
        "id_token": assertion.id_token.expose_secret(),
        "refresh_token": assertion.refresh_token.as_ref().map(|token| token.expose_secret()),
        "issuer": assertion.issuer,
        "expires_at_ms": assertion.expires_at.map(|moment| moment.timestamp_millis()),
    })
}

pub(crate) struct Transcript {
    pub(crate) url: Option<String>,
    pub(crate) form: Vec<[String; 2]>,
    pub(crate) headers: Vec<[String; 2]>,
    pub(crate) writes: Vec<Value>,
    pub(crate) posts: usize,
}

fn pairs(map: &BTreeMap<String, String>) -> Vec<[String; 2]> {
    map.iter()
        .map(|(name, value)| [name.clone(), value.clone()])
        .collect()
}

pub(crate) fn transcript(
    transport: &ScriptedTransport,
    writes: &Mutex<Vec<(String, SsoIdentityAssertion)>>,
) -> Transcript {
    let calls = transport
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let first = calls.first();
    Transcript {
        url: first.map(|call| call.url.clone()),
        form: first.map_or_else(Vec::new, |call| pairs(&call.form)),
        headers: first.map_or_else(Vec::new, |call| pairs(&call.headers)),
        writes: writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(user_id, assertion)| json!({"user_id": user_id, "assertion": view(assertion)}))
            .collect(),
        posts: calls.len(),
    }
}

pub(crate) fn kind_text(kind: RefreshFailureKind) -> &'static str {
    match kind {
        RefreshFailureKind::Rejected => "rejected",
        RefreshFailureKind::Unavailable => "unavailable",
    }
}

pub(crate) fn answered(
    result: &str,
    detail: &Value,
    assertion: &Value,
    transcript: &Transcript,
) -> Value {
    json!({
        "result": result,
        "detail": detail,
        "assertion": assertion,
        "url": transcript.url,
        "form": transcript.form,
        "headers": transcript.headers,
        "writes": transcript.writes,
        "posts": transcript.posts,
    })
}
