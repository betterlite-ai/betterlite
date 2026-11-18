use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_anthropic::batches::config::AnthropicBatchesConfig;
use betterlite_anthropic::batches::handler::AnthropicBatchesHandler;
use betterlite_anthropic::batches::timestamps::Clock;
use betterlite_anthropic::count_tokens::handler::AnthropicCountTokensHandler;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::files::api::FilesApi;
use betterlite_anthropic::files::request::FilesClient;
use chrono::FixedOffset;

use super::Input;
use super::recording::Transport;

pub(super) struct Services {
    pub(super) count: AnthropicCountTokensHandler<Transport>,
    pub(super) batches: AnthropicBatchesHandler<Transport>,
    pub(super) files: FilesApi<Transport>,
    pub(super) client: FilesClient,
}

const fn default_now() -> i64 {
    1_700_000_000
}

impl From<&Input> for Services {
    // Keep the fixture dependency table together.
    #[rustfmt::skip]
    fn from(input: &Input) -> Self {
        let transport = Transport {
            sent: Arc::new(Mutex::new(None)), response: input.response.clone(), expected_key: input.expected_key.clone(),
        };
        let settings = AnthropicSettings::default();
        Self {
            count: AnthropicCountTokensHandler { transport: transport.clone(), request_timeout: Duration::from_secs(5), beta_version: "token-counting-2024-11-01".to_owned() },
            batches: AnthropicBatchesHandler {
                transport: transport.clone(), config: AnthropicBatchesConfig { settings: settings.clone() },
                clock: Clock { now: default_now(), zone: FixedOffset::default() }, request_timeout: Duration::from_secs(5),
            },
            files: FilesApi { transport, clock: default_now },
            client: FilesClient { api_key: input.api_key.clone(), api_base: input.api_base.clone(), settings, timeout: Duration::from_secs(5) },
        }
    }
}
