use std::borrow::Cow;
use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_models::decisions::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;

use crate::gemini_history::convert::convert_messages_with_history;
use crate::google_count_tokens::decode::GoogleCountResponse;
use crate::google_count_tokens::transport::CountTransport;
use crate::google_count_tokens::vertex::{VertexCountInput, count_tokens};
use crate::google_token_counter::counter::{CountRuntime, CountTokensCall};
use crate::google_token_counter::error::CountTokensError;
use crate::google_token_counter::reply::{CountReply, TokenCountResponse, counted};
use crate::vertex_interactions::mint::MintAccessToken;

// LiteLLM's non-partner branch: supplied contents win even when empty, and only absent contents convert the messages
pub(crate) async fn count<T, M, F>(
    runtime: &CountRuntime<'_, T, M, F>,
    settings: &VertexSettings,
    call: &CountTokensCall<'_>,
    deadline: Duration,
) -> Result<Option<TokenCountResponse>, CountTokensError>
where
    T: CountTransport,
    M: MintAccessToken,
    F: RemoteMediaFetcher + Sync,
{
    let transport = runtime.transport;
    let refresh = runtime.refresh;
    let catalog = runtime.catalog;
    let fetcher = runtime.fetcher;
    let contents: Cow<'_, Json> = match call
        .contents
        .filter(|contents| !matches!(contents, Json::Null))
    {
        Some(contents) => Cow::Borrowed(contents),
        None => Cow::Owned(history_contents(call, fetcher).await?),
    };
    let headers = BTreeMap::new();
    let input = VertexCountInput {
        model: call.model_to_use,
        contents: &contents,
        api_key: call.deployment.api_key,
        api_base: call.deployment.api_base,
        headers: &headers,
        params: call.deployment.vertex,
        settings,
        timeout: call.deployment.timeout,
    };
    let reply = count_tokens(transport, refresh, catalog, &input, deadline).await?;
    reply_with_total(call, reply)
}

async fn history_contents<F>(
    call: &CountTokensCall<'_>,
    fetcher: &F,
) -> Result<Json, CountTokensError>
where
    F: RemoteMediaFetcher + Sync,
{
    let history = convert_messages_with_history(call.messages, None, fetcher).await?;
    serde_json::to_value(&history.contents)
        .and_then(serde_json::from_value::<Json>)
        .map_err(|source| CountTokensError::Message { source })
}

// LiteLLM wraps a Vertex reply only when it carries totalTokens, so one without it counts nothing at all
fn reply_with_total(
    call: &CountTokensCall<'_>,
    reply: GoogleCountResponse,
) -> Result<Option<TokenCountResponse>, CountTokensError> {
    let Some(total) = reply.total_tokens else {
        return Ok(None);
    };
    let tokens = counted(total)?;
    let tokenizer_type = reply.tokenizer_used.clone().unwrap_or_default();
    Ok(Some(TokenCountResponse::new(
        call,
        tokens,
        tokenizer_type,
        CountReply::Google(Box::new(reply)),
    )))
}
