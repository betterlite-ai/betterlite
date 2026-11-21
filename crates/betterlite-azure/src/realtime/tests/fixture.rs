use betterlite_config::tls::Verification;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::realtime::url::QueryParams;
use secrecy::SecretString;

use crate::realtime::connect::{
    RealtimeConnectSettings, RealtimeConnectionPlan, prepare_connection,
};
use crate::realtime::error::RealtimeError;
use crate::realtime::http_config::RealtimeHttpSettings;

pub(super) const MODEL: &str = "realtime-a";
pub(super) const KEY: &str = "azure-key";
pub(super) const TOKEN: &str = "ad-token";
pub(super) const MAX_SIZE: usize = 8_192;
pub(super) const API_VERSION: &str = "2024-10-01-preview";

pub(super) fn params(pairs: &[(&str, Option<&str>)]) -> QueryParams {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.map(str::to_owned)))
        .collect()
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn connection<'base>(
    api_base: Option<&'base str>,
    api_version: Option<&'base str>,
    protocol: Option<&'base str>,
    params: Option<&'base QueryParams>,
) -> Connection<'base> {
    Connection {
        api_base,
        api_version,
        protocol,
        params,
        key: Some(SecretString::from(KEY.to_owned())),
        token: None,
        verification: Verification::Enabled,
    }
}

pub(super) struct Connection<'base> {
    api_base: Option<&'base str>,
    api_version: Option<&'base str>,
    protocol: Option<&'base str>,
    params: Option<&'base QueryParams>,
    key: Option<SecretString>,
    token: Option<SecretString>,
    verification: Verification,
}

impl Connection<'_> {
    pub(super) fn with_key(mut self, key: Option<&str>) -> Self {
        self.key = key.map(SecretString::from);
        self
    }

    pub(super) fn with_token(mut self, token: Option<&str>) -> Self {
        self.token = token.map(SecretString::from);
        self
    }

    pub(super) fn with_verification(mut self, verification: Verification) -> Self {
        self.verification = verification;
        self
    }

    fn settings(&self) -> RealtimeConnectSettings<'_> {
        RealtimeConnectSettings {
            api_base: self.api_base,
            api_key: self.key.as_ref(),
            azure_ad_token: self.token.as_ref(),
            model: MODEL,
            api_version: self.api_version,
            realtime_protocol: self.protocol,
            query_params: self.params,
            ssl_verification: &self.verification,
            max_message_size_bytes: MAX_SIZE,
        }
    }

    pub(super) fn outcome(&self) -> Result<RealtimeConnectionPlan, RealtimeError> {
        prepare_connection(&self.settings())
    }

    pub(super) fn plan(&self) -> RealtimeConnectionPlan {
        self.outcome().expect("the connection plan is built")
    }

    pub(super) fn url(&self) -> String {
        self.plan().url
    }

    pub(super) fn error(&self) -> RealtimeError {
        self.outcome().err().expect("the connection is refused")
    }
}

pub(super) fn http_settings(
    global_base: Option<&str>,
    global_key: Option<&str>,
    env_base: Option<&str>,
    env_key: Option<&str>,
) -> RealtimeHttpSettings {
    RealtimeHttpSettings {
        litellm_api_base: global_base.map(str::to_owned),
        litellm_api_key: global_key.map(SecretString::from),
        azure_api_base: env_base.map(str::to_owned),
        azure_api_key: env_key.map(SecretString::from),
        azure_api_version: None,
    }
}

pub(super) fn with_version(
    mut settings: RealtimeHttpSettings,
    version: Option<&str>,
) -> RealtimeHttpSettings {
    settings.azure_api_version = version.map(str::to_owned);
    settings
}
