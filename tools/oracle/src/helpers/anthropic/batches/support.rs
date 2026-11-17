use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_anthropic::batches::handler::{BatchTransport, default_batch_logging};
use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_config::sections::anthropic_credentials::Settings as ConfigSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

pub(crate) const CLIENT_TIMEOUT: Duration = Duration::from_secs(600);
pub(crate) const EXPLICIT_HEADERS: [&str; 4] = [
    "accept",
    "anthropic-beta",
    "anthropic-version",
    "content-type",
];
pub(crate) const AUTH_HEADERS: [&str; 2] = ["authorization", "x-api-key"];

pub(crate) fn settings(
    environment: &BTreeMap<String, String>,
) -> Result<AnthropicSettings, Box<HelperError>> {
    let resolved = ConfigSettings::resolve_environment(environment).map_err(failure)?;
    Ok(AnthropicSettings {
        api_key: resolved.api_key,
        auth_token: resolved.auth_token,
        api_base: resolved.api_base,
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    })
}

pub(crate) fn header_map(pairs: &[(String, String)]) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let header_name = HeaderName::try_from(name.as_str()).map_err(failure)?;
        let header_value = HeaderValue::from_str(value).map_err(failure)?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

pub(crate) fn header_fields(headers: &HeaderMap, names: Option<&[&str]>) -> Value {
    let selected: Vec<(&str, &HeaderValue)> = names.map_or_else(
        || {
            headers
                .iter()
                .filter(|(name, _value)| !matches!(name.as_str(), "authorization" | "x-api-key"))
                .map(|(name, value)| (name.as_str(), value))
                .collect()
        },
        |keep| {
            keep.iter()
                .filter_map(|name| headers.get(*name).map(|value| (*name, value)))
                .collect()
        },
    );
    let mut fields = BTreeMap::new();
    for (name, value) in selected {
        let text = value
            .to_str()
            .map_or_else(|_invalid| String::new(), str::to_owned);
        fields.insert(name.to_owned(), text);
    }
    json!(fields)
}

pub(crate) fn instant(now: i64) -> Result<DateTime<FixedOffset>, Box<HelperError>> {
    let at = DateTime::from_timestamp(now, 0)
        .ok_or_else(|| failure("the fixture now is outside the representable range"))?;
    Ok(at.fixed_offset())
}

pub(crate) fn utc_offset() -> Result<FixedOffset, Box<HelperError>> {
    let Some(offset) = FixedOffset::east_opt(0) else {
        return Err(failure("the UTC offset is not representable"));
    };
    Ok(offset)
}

pub(crate) fn logging_state(
    batch_id: &str,
    now: i64,
    redactor: &Redactor,
) -> Result<LoggingState, Box<HelperError>> {
    let init = default_batch_logging(batch_id).into_init(instant(now)?);
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "batch-trace".to_owned(),
        Duration::default,
    )
    .or_config_error()
}

pub(crate) fn logging_env<'env>(
    now: i64,
    redactor: &'env Redactor,
    sink: &'env Silent,
) -> Result<PrePostEnv<'env>, Box<HelperError>> {
    let at = instant(now)?;
    Ok(PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: at,
    })
}

pub(crate) fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "Error".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(crate) struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct ZeroCounter;

impl ReasoningTokenCounter for ZeroCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Ok(0)
    }
}

pub(crate) struct Replay {
    pub(crate) status: u16,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
    pub(crate) seen: Mutex<Vec<OutboundRequest>>,
}

impl Replay {
    pub(crate) fn new(
        status: u16,
        pairs: &[(String, String)],
        body: &str,
    ) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            status,
            headers: header_map(pairs)?,
            body: body.as_bytes().to_vec(),
            seen: Mutex::new(Vec::new()),
        })
    }
}

impl BatchTransport for &Replay {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        }))
    }
}
