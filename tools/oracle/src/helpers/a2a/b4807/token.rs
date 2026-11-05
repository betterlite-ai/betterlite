use std::convert::Infallible;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_a2a::watsonx_credentials::bearer::{
    BearerTokens, CredentialsError, TokenClock, TokenHttpResponse, TokenRequest, TokenTransport,
};
use betterlite_a2a::watsonx_credentials::params::WxoRequestParams;
use betterlite_a2a::watsonx_credentials::token_cache::{BoundedTokenCache, TokenStore};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use url::form_urlencoded;

use super::failure;
use crate::helpers::HelperResult;

#[derive(Deserialize)]
pub(crate) struct Step {
    params: StepParams,
    at: u64,
    wall: i64,
    #[serde(default)]
    answer: Value,
}

#[derive(Deserialize)]
struct StepParams {
    cp4d_host: String,
    auth_mode: String,
    api_key: SecretString,
    username: Option<String>,
}

struct StepClock {
    at: Duration,
    wall: i64,
}

impl TokenClock for StepClock {
    fn monotonic(&self) -> Duration {
        self.at
    }
    fn unix_seconds(&self) -> i64 {
        self.wall
    }
}

struct SharedStore(Arc<BoundedTokenCache>);

impl TokenStore for SharedStore {
    fn lookup(&self, key: &str, now: Duration) -> Option<SecretString> {
        self.0.lookup(key, now)
    }
    fn insert(&self, key: String, token: SecretString, expires_at: Duration) {
        self.0.insert(key, token, expires_at);
    }
    fn evict_expired(&self, now: Duration) {
        self.0.evict_expired(now);
    }
}

struct Recorder<'seen> {
    answer: Vec<u8>,
    seen: &'seen Mutex<Vec<Value>>,
}

fn record(request: &TokenRequest) -> Value {
    let body = request.body.expose_secret();
    let (data, json_body) = if request.content_type == "application/json" {
        (
            Value::Null,
            serde_json::from_slice(body).unwrap_or(Value::Null),
        )
    } else {
        let fields = form_urlencoded::parse(body)
            .map(|(key, value)| (key.into_owned(), Value::String(value.into_owned())))
            .collect::<Map<_, _>>();
        (Value::Object(fields), Value::Null)
    };
    json!({
        "url": request.url,
        "data": data,
        "json": json_body,
        "headers": {"Content-Type": request.content_type},
    })
}

impl TokenTransport for Recorder<'_> {
    type Error = Infallible;

    fn post(
        &self,
        request: &TokenRequest,
    ) -> impl Future<Output = Result<TokenHttpResponse, Self::Error>> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record(request));
        let response = TokenHttpResponse {
            status: 200,
            body: self.answer.clone(),
        };
        async move { Ok(response) }
    }
}

fn run_step(
    runtime: &tokio::runtime::Runtime,
    store: &Arc<BoundedTokenCache>,
    step: &Step,
) -> Value {
    let seen = Mutex::new(Vec::new());
    let tokens = BearerTokens {
        transport: Recorder {
            answer: serde_json::to_vec(&step.answer).unwrap_or_default(),
            seen: &seen,
        },
        clock: StepClock {
            at: Duration::from_secs(step.at),
            wall: step.wall,
        },
        store: SharedStore(Arc::clone(store)),
    };
    let params = WxoRequestParams {
        cp4d_host: step.params.cp4d_host.clone(),
        instance_id: String::new(),
        wxo_agent_id: String::new(),
        api_key: SecretString::from(step.params.api_key.expose_secret()),
        username: step.params.username.clone(),
        auth_mode: step.params.auth_mode.clone(),
        thread_id: None,
    };
    match runtime.block_on(tokens.bearer_token(&params)) {
        Ok(token) => json!({
            "token": token.expose_secret(),
            "requests": seen.into_inner().unwrap_or_else(PoisonError::into_inner),
        }),
        Err(error @ CredentialsError::MissingUsername) => {
            json!({"error": {"class": "ValueError", "message": error.to_string()}})
        }
        Err(error) => json!({"error": {"class": "TokenError", "message": error.to_string()}}),
    }
}

pub(super) fn run_steps(steps: &[Step]) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| failure("RuntimeError", error.to_string()))?;
    let store = Arc::new(BoundedTokenCache::new(NonZeroUsize::MIN));
    let outcomes = steps
        .iter()
        .map(|step| run_step(&runtime, &store, step))
        .collect();
    Ok(Value::Array(outcomes))
}
