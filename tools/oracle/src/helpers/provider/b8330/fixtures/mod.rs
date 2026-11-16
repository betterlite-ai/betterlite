use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_provider::data_cloud_dispatch::error::DispatchError;
use betterlite_provider::data_cloud_dispatch::transport::{PostTransport, StreamHead, WireRequest};
use betterlite_provider::databricks_auth::oauth::{
    OauthReply, OauthTokenRequest, OauthTokenSource,
};
use betterlite_provider::oci_common::Clock;
use betterlite_redact::secret::Redactor;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use chrono::{DateTime, FixedOffset, Utc};
use http::HeaderMap;
use secrecy::SecretString;

use super::input::Case;

// the placeholder a case uses for the workspace it cannot know when it is written
pub(super) const ORACLE_UPSTREAM: &str = "https://workspace.example.com";
const INIT: &str = r#"{"model":"oracle","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW_SECONDS: i64 = 1_700_000_005;

struct Offline;

impl OidcHttp for Offline {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the b8330 fixture holds no OIDC transport"),
        })
    }
}

impl AzureOidcTokens for Offline {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the b8330 fixture holds no Azure token source"),
        })
    }
}

impl OidcClock for Offline {
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(Offline),
        azure_tokens: Arc::new(Offline),
        clock: Arc::new(Offline),
        oidc_cache: OidcTokenCache::default(),
    }
}

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging(
    redactor: &Redactor,
) -> Result<(LoggingState, PrePostEnv<'_>), Box<HelperError>> {
    let init = serde_json::from_str::<LoggingInit>(INIT).or_value_error()?;
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .or_value_error()?;
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags {
            log_raw_request_response: true,
            turn_off_message_logging: false,
        },
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Discard,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::<FixedOffset>::from(
            DateTime::from_timestamp(NOW_SECONDS, 0).unwrap_or_default(),
        ),
    };
    Ok((state, env))
}

pub(super) fn environment(case: &Case) -> BTreeMap<String, String> {
    case.env
        .iter()
        .map(|(name, value)| (name.clone(), value.replace("{{upstream}}", ORACLE_UPSTREAM)))
        .collect()
}

#[derive(Clone)]
pub(super) struct Sent {
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

impl Sent {
    fn record(recorded: &Mutex<Vec<Self>>, request: &WireRequest<'_>) {
        recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Self {
                url: request.url.to_owned(),
                headers: request.headers.clone(),
                body: request.body.to_vec(),
            });
    }
}

pub(super) struct Scripted {
    replies: Mutex<VecDeque<UpstreamResponse>>,
    streams: Mutex<VecDeque<StreamHead>>,
    recorded: Mutex<Vec<Sent>>,
}

impl Scripted {
    pub(super) fn with_replies(replies: Vec<UpstreamResponse>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            streams: Mutex::new(VecDeque::new()),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn with_stream(head: StreamHead) -> Self {
        Self {
            replies: Mutex::new(VecDeque::new()),
            streams: Mutex::new(VecDeque::from([head])),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn first(&self) -> Option<Sent> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
    }
}

fn no_reply() -> DispatchError {
    DispatchError::Upstream {
        status: 599,
        body: String::from("the b8330 fixture holds no reply"),
    }
}

fn take<T>(queue: &Mutex<VecDeque<T>>) -> Result<T, DispatchError> {
    queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop_front()
        .ok_or_else(no_reply)
}

#[async_trait]
impl PostTransport for Scripted {
    async fn post(&self, request: &WireRequest<'_>) -> Result<UpstreamResponse, DispatchError> {
        Sent::record(&self.recorded, request);
        take(&self.replies)
    }

    async fn post_stream(&self, request: &WireRequest<'_>) -> Result<StreamHead, DispatchError> {
        Sent::record(&self.recorded, request);
        take(&self.streams)
    }
}

pub(super) struct NoTokens;

#[async_trait]
impl OauthTokenSource for NoTokens {
    async fn send(&self, _request: &OauthTokenRequest) -> OauthReply {
        OauthReply::Transport(String::from("the b8330 fixture holds no token source"))
    }
}

pub(super) struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap_or_default()
    }
}
