use betterlite_config::tls::Verification;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;

use crate::openai_base_config::Headers;
use crate::realtime::connect::{
    RealtimeConnectSettings, RealtimeConnectionPlan, prepare_connection,
};
use crate::realtime::error::RealtimeError;
use crate::realtime::http_config::RealtimeHttpSettings;
use crate::realtime::logging::{RealtimeLogRecord, RealtimeLogger};
use crate::realtime::url::QueryParams;

pub(super) const MODEL: &str = "realtime-a";
pub(super) const KEY: &str = "sk-test";
pub(super) const MAX_SIZE: usize = 4096;
pub(super) const DEFAULT_URL: &str = "wss://api.openai.com/v1/realtime?model=realtime-a";

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

pub(super) fn plan_for(
    api_base: Option<&str>,
    query_params: Option<&QueryParams>,
    ssl_verification: &Verification,
) -> RealtimeConnectionPlan {
    prepare(
        api_base,
        query_params,
        false,
        ssl_verification,
        &mut RecordingLogger::default(),
    )
    .unwrap()
}

pub(super) fn url_for(api_base: Option<&str>, query_params: Option<&QueryParams>) -> String {
    plan_for(api_base, query_params, &Verification::Enabled).url
}

pub(super) fn logged_plan(
    api_base: Option<&str>,
    query_params: Option<&QueryParams>,
    beta_realtime: bool,
) -> (RealtimeConnectionPlan, RecordingLogger) {
    let mut logger = RecordingLogger::default();
    let plan = prepare(
        api_base,
        query_params,
        beta_realtime,
        &Verification::Enabled,
        &mut logger,
    )
    .unwrap();
    (plan, logger)
}

pub(super) fn http_bases(
    global_base: Option<&str>,
    env_base: Option<&str>,
) -> RealtimeHttpSettings {
    RealtimeHttpSettings {
        litellm_api_base: global_base.map(str::to_owned),
        litellm_api_key: None,
        litellm_openai_key: None,
        openai_api_base: env_base.map(str::to_owned),
        openai_api_key: None,
    }
}

pub(super) fn http_auth_settings(
    global_key: Option<&str>,
    realtime_key: Option<&str>,
    env_key: Option<&str>,
) -> RealtimeHttpSettings {
    RealtimeHttpSettings {
        litellm_api_base: None,
        litellm_api_key: global_key.map(SecretString::from),
        litellm_openai_key: realtime_key.map(SecretString::from),
        openai_api_base: None,
        openai_api_key: env_key.map(SecretString::from),
    }
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
