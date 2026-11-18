use super::{TIMEOUT, fixture::Fixture};
use betterlite_anthropic::batches::{
    config::AnthropicBatchesConfig, handler::AnthropicBatchesHandler, timestamps::Clock,
};
use betterlite_anthropic::count_tokens::handler::AnthropicCountTokensHandler;
use betterlite_anthropic::files::{api::FilesApi, request::FilesClient};
use betterlite_http::proxy::ProxiedClient;
use chrono::{Offset as _, Utc};

pub(super) struct Services {
    pub(super) count: AnthropicCountTokensHandler<ProxiedClient>,
    pub(super) batches: AnthropicBatchesHandler<ProxiedClient>,
    pub(super) files: FilesApi<ProxiedClient>,
    pub(super) file_client: FilesClient,
}
impl From<&Fixture> for Services {
    fn from(fixture: &Fixture) -> Self {
        Self {
            count: AnthropicCountTokensHandler {
                transport: fixture.client.clone(),
                request_timeout: TIMEOUT,
                beta_version: "token-counting-2024-11-01".to_owned(),
            },
            batches: AnthropicBatchesHandler {
                transport: fixture.client.clone(),
                config: AnthropicBatchesConfig {
                    settings: fixture.settings.clone(),
                },
                clock: Clock {
                    now: 1_700_000_000,
                    zone: Utc.fix(),
                },
                request_timeout: TIMEOUT,
            },
            files: FilesApi {
                transport: fixture.client.clone(),
                clock: || 1_700_000_000,
            },
            file_client: FilesClient {
                api_key: None,
                api_base: None,
                settings: fixture.settings.clone(),
                timeout: TIMEOUT,
            },
        }
    }
}
