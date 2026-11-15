use betterlite_config::tls::Verification;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;

use crate::openai_base_config::Headers;
use crate::realtime::connect::{
    RealtimeConnectSettings, RealtimeConnectionPlan, prepare_connection,
};
use crate::realtime::error::RealtimeError;
use crate::realtime::logging::{RealtimeLogRecord, RealtimeLogger};
use crate::realtime::url::QueryParams;

pub(super) const MODEL: &str = "realtime-a";
pub(super) const KEY: &str = "sk-test";
pub(super) const MAX_SIZE: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RecordedCall {
    pub(super) api_base: String,
    pub(super) headers: Headers,
    pub(super) query_params: QueryParams,
}

#[derive(Default)]
pub(super) struct RecordingLogger {
    pub(super) calls: Vec<RecordedCall>,
}

impl RealtimeLogger for RecordingLogger {
    fn pre_call(&mut self, record: RealtimeLogRecord<'_>) -> Result<(), PrePostError> {
        self.calls.push(RecordedCall {
            api_base: record.api_base.to_owned(),
            headers: record.headers.clone(),
            query_params: record.query_params.clone(),
        });
        Ok(())
    }
}

pub(super) fn prepare(
    api_base: Option<&str>,
    query_params: Option<&QueryParams>,
    beta_realtime: bool,
    ssl_verification: &Verification,
    logger: &mut RecordingLogger,
) -> Result<RealtimeConnectionPlan, RealtimeError> {
    let key = SecretString::from(KEY.to_owned());
    let settings = RealtimeConnectSettings {
        api_base,
        api_key: Some(&key),
        model: MODEL,
        query_params,
        beta_realtime,
        ssl_verification,
        max_message_size_bytes: MAX_SIZE,
    };
    prepare_connection(&settings, logger)
}

pub(super) fn params(pairs: &[(&str, Option<&str>)]) -> QueryParams {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.map(str::to_owned)))
        .collect()
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds")
}
