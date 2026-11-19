use betterlite_models::decisions::Json;
use secrecy::SecretString;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_prediction_targets::counting::PromptCount;
use crate::cache_prediction_targets::counting::count_prompt_tokens;
use crate::cache_prediction_targets::messages_url::MessagesUrl;
use crate::count_tokens::handler::AnthropicCountTokensHandler;
use crate::count_tokens::handler::CountTokensTransport;

impl AnthropicCacheRouting {
    pub async fn count_tokens<Transport, Builder>(
        handler: &AnthropicCountTokensHandler<Transport>,
        messages_url: &Builder,
        model: &str,
        api_key: &SecretString,
        body: &Json,
    ) -> Option<i64>
    where
        Transport: CountTokensTransport,
        Builder: MessagesUrl,
    {
        count_prompt_tokens(
            handler,
            messages_url,
            &PromptCount {
                model,
                api_key,
                body,
                api_base: None,
            },
        )
        .await
    }
}

mod tests;
