use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_bedrock::embedding_jobs::logging::{StatusEvent, StatusLogger, log_call};
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
};
use secrecy::SecretString;

use super::Failure;

pub(crate) struct NoPorts;

impl OidcHttp for NoPorts {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoPorts {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoPorts {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

#[derive(Default)]
pub(crate) struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) fn logging_state(redactor: &Redactor) -> Result<LoggingState, Failure> {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"embedding-model","call_type":"embeddings","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated".to_owned(),
        Duration::default,
    )
    .map_err(Failure::from)
}

pub(crate) fn with_logging<Invoke, Output>(
    enabled: bool,
    invoke: Invoke,
) -> Result<(Output, Vec<Json>), Failure>
where
    Invoke: FnOnce(Option<&mut StatusLogger<'_>>) -> Output,
{
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(Failure::from)?;
    let mut state = logging_state(&redactor)?;
    let snapshots = Mutex::new(Vec::new());
    let mut logger_callback = |arn: &str, event: StatusEvent<'_>| {
        capture_status_event(&mut state, &redactor, &snapshots, arn, event)
    };
    let result = invoke(enabled.then_some(&mut logger_callback));
    let captured = snapshots
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    Ok((result, captured))
}

fn capture_status_event(
    state: &mut LoggingState,
    redactor: &Redactor,
    snapshots: &Mutex<Vec<Json>>,
    arn: &str,
    event: StatusEvent<'_>,
) -> Result<(), PrePostError> {
    let logger = |snapshot: &Json| -> Result<(), PrePostError> {
        snapshots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(snapshot.clone());
        Ok(())
    };
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &DiscardingSink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: Some(&logger),
        now: state.start_time,
    };
    log_call(arn, event, &mut LogContext { state, env: &env })
}
