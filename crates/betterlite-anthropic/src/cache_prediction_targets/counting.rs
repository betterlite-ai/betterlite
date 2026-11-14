use std::future::Future;
use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::time::timeout;

use crate::cache_prediction_targets::messages_url::MessagesUrl;
use crate::count_tokens::handler::{AnthropicCountTokensHandler, CountTokensTransport};
use crate::count_tokens::wire::{CountTokensBody, CountTokensRequest};

const COUNT_TIMEOUT: Duration = Duration::from_secs(15);
const COUNT_OPTION_NAMES: [&str; 3] = ["thinking", "tool_choice", "output_config"];

pub struct PromptCount<'count> {
    pub model: &'count str,
    pub api_key: &'count SecretString,
    pub body: &'count Json,
    pub api_base: Option<&'count str>,
}

pub trait TokenCounter: Sync {
    fn count(
        &self,
        model: &str,
        api_key: &SecretString,
        body: &Json,
    ) -> impl Future<Output = Option<i64>> + Send;
}

pub struct PromptTokenCounter<'deps, Transport, Builder> {
    pub handler: &'deps AnthropicCountTokensHandler<Transport>,
    pub messages_url: &'deps Builder,
}

impl<Transport, Builder> TokenCounter for PromptTokenCounter<'_, Transport, Builder>
where
    Transport: CountTokensTransport,
    Builder: MessagesUrl,
{
    async fn count(&self, model: &str, api_key: &SecretString, body: &Json) -> Option<i64> {
        let call = PromptCount {
            model,
            api_key,
            body,
            api_base: None,
        };
        count_prompt_tokens(self.handler, self.messages_url, &call).await
    }
}

pub async fn count_prompt_tokens<Transport, Builder>(
    handler: &AnthropicCountTokensHandler<Transport>,
    messages_url: &Builder,
    call: &PromptCount<'_>,
) -> Option<i64>
where
    Transport: CountTokensTransport,
    Builder: MessagesUrl,
{
    let body = count_body(call.body)?;
    let endpoint = messages_url.messages_url(call.api_base);
    let count_url = format!("{endpoint}/count_tokens");
    let request = CountTokensRequest {
        body: CountTokensBody {
            model: call.model,
            messages: body.messages,
            tools: body.tools,
            system: body.system,
            optional_params: &body.options,
        },
        api_key: call.api_key,
        api_base: Some(&count_url),
        timeout: Some(COUNT_TIMEOUT),
    };
    // the whole count, not one provider read, has to finish inside the 15-second timeout
    let response = timeout(COUNT_TIMEOUT, handler.handle_count_tokens_request(&request))
        .await
        .ok()?
        .ok()?;
    let Some(Json::Number(number)) = response.get("input_tokens") else {
        return None;
    };
    number.as_i64().filter(|tokens| *tokens >= 0)
}

struct CountBody<'body> {
    messages: &'body Json,
    tools: Option<&'body Json>,
    system: Option<&'body Json>,
    options: IndexMap<String, Json>,
}

fn count_body(body: &Json) -> Option<CountBody<'_>> {
    let Json::Object(fields) = body else {
        return None;
    };
    let messages = fields.get("messages")?;
    if !is_mapping_list(messages) {
        return None;
    }
    let tools = match fields.get("tools") {
        None | Some(Json::Null) => None,
        Some(value) if is_mapping_list(value) => Some(value),
        Some(_) => return None,
    };
    let system = match fields.get("system") {
        None | Some(Json::Null) => None,
        Some(value @ Json::Text(_)) => Some(value),
        Some(value) if is_mapping_list(value) => Some(value),
        Some(_) => return None,
    };
    let mut options = IndexMap::new();
    for name in COUNT_OPTION_NAMES {
        let Some(value) = fields.get(name) else {
            continue;
        };
        if !matches!(value, Json::Null | Json::Object(_)) {
            return None;
        }
        options.insert(name.to_owned(), value.clone());
    }
    Some(CountBody {
        messages,
        tools,
        system,
        options,
    })
}

fn is_mapping_list(value: &Json) -> bool {
    match value {
        Json::List(items) => items.iter().all(|item| matches!(item, Json::Object(_))),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => false,
    }
}
