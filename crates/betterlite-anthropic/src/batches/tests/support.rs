use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::sections::anthropic_credentials::DEFAULT_API_BASE;
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
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;

use crate::batches::config::AnthropicBatchesConfig;
use crate::batches::handler::{AnthropicBatchesHandler, BatchTransport, default_batch_logging};
use crate::batches::timestamps::Clock;
use crate::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use crate::usage::ReasoningTokenCounter;

pub(super) const NOW: i64 = 1_704_067_200;
pub(super) const ZONE_SECONDS: i32 = 7_200;
pub(super) const TIMEOUT: Duration = Duration::from_secs(7);

pub(super) fn clock() -> Clock {
    Clock {
        now: NOW,
        zone: FixedOffset::east_opt(ZONE_SECONDS).unwrap(),
    }
}

pub(super) fn instant() -> DateTime<FixedOffset> {
    DateTime::from_timestamp(NOW, 0).unwrap().fixed_offset()
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn configured(
    api_key: Option<&str>,
    auth_token: Option<&str>,
    api_base: Option<&str>,
) -> AnthropicSettings {
    AnthropicSettings {
        api_key: api_key.map(secret),
        auth_token: auth_token.map(secret),
        api_base: api_base.map_or_else(|| DEFAULT_API_BASE.to_owned(), str::to_owned),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}

pub(super) fn config(api_key: Option<&str>, api_base: Option<&str>) -> AnthropicBatchesConfig {
    AnthropicBatchesConfig {
        settings: configured(api_key, None, api_base),
    }
}

#[derive(Debug)]
pub(super) struct Fixture {
    pub(super) status: u16,
    pub(super) headers: Vec<(&'static str, &'static str)>,
    pub(super) body: Vec<u8>,
    pub(super) fail: bool,
    pub(super) seen: Mutex<Vec<OutboundRequest>>,
}

impl Fixture {
    pub(super) fn answering(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.to_vec(),
            fail: false,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn answering_with(
        status: u16,
        headers: &[(&'static str, &'static str)],
        body: &[u8],
    ) -> Self {
        let mut fixture = Self::answering(status, body);
        fixture.headers = headers.to_vec();
        fixture
    }

    pub(super) fn failing() -> Self {
        Self {
            status: 0,
            headers: Vec::new(),
            body: Vec::new(),
            fail: true,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn reply(&self) -> UpstreamResponse {
        let mut headers = HeaderMap::new();
        for (name, value) in &self.headers {
            headers.insert(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }
        UpstreamResponse {
            status: self.status,
            headers,
            body: self.body.clone(),
        }
    }

    pub(super) fn only_request(&self) -> OutboundRequest {
        let mut seen = self.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "the call sends exactly one request");
        seen.remove(0)
    }

    pub(super) fn sent(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    pub(super) fn take_requests(&self) -> Vec<OutboundRequest> {
        std::mem::take(&mut *self.seen.lock().unwrap())
    }
}

impl BatchTransport for &Fixture {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.seen.lock().unwrap().push(request);
        if self.fail {
            return ready(Err(TransportError::Closed));
        }
        ready(Ok(self.reply()))
    }
}

pub(super) fn batches(
    transport: &Fixture,
    config: AnthropicBatchesConfig,
) -> AnthropicBatchesHandler<&Fixture> {
    AnthropicBatchesHandler {
        transport,
        config,
        clock: clock(),
        request_timeout: TIMEOUT,
    }
}

pub(super) fn logging_state(batch_id: &str, redactor: &Redactor) -> LoggingState {
    let init = default_batch_logging(batch_id).into_init(instant());
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "trace-b587".to_owned(),
        || Duration::default(),
    )
    .unwrap()
}

pub(super) fn logging_env<'env>(
    redactor: &'env Redactor,
    silent: &'env Silent,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: instant(),
    }
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
}

pub(super) struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct FixedCounter;

impl ReasoningTokenCounter for FixedCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Ok(0)
    }
}

pub(super) fn header_map(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    headers
}

pub(super) fn header(headers: &HeaderMap, name: &str) -> String {
    let value = headers.get(name).unwrap();
    value
        .to_str()
        .map_or_else(|_invalid| String::new(), str::to_owned)
}

pub(super) fn has_header(headers: &HeaderMap, name: &str) -> bool {
    headers.contains_key(name)
}
