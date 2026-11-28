use betterlite_models::decisions::{Extra, Json};
use betterlite_prompts::arguments::json::Object;

use crate::api::messages_compaction::convert::prompt_json;
use crate::api::summary_execution::completion::SummaryCompletion;

// the content block type Anthropic clients use for a client-side summary
const COMPACTION_BLOCK_TYPE: &str = "compaction";

// what the proxy leaves in kwargs["litellm_metadata"] for /v1/messages: the map itself and its auth entry
pub struct ProxyContext {
    pub litellm_metadata: Object,
    pub user_api_key_auth: Option<Json>,
}

// _extract_proxy_litellm_metadata: a mapping under litellm_metadata yields its fields, anything else yields nothing
#[must_use]
pub fn extract_proxy_metadata(kwargs: &Extra) -> Option<ProxyContext> {
    let Some(Json::Object(fields)) = kwargs.get("litellm_metadata") else {
        return None;
    };
    let mut metadata = Object::new();
    for (key, value) in fields {
        metadata.insert(key.clone(), prompt_json(value));
    }
    Some(ProxyContext {
        litellm_metadata: metadata,
        user_api_key_auth: fields.get("user_api_key_auth").cloned(),
    })
}

// _messages_have_compaction_block: a compaction dict block inside a list content on any role
#[must_use]
pub fn has_compaction_block(messages: &[Json]) -> bool {
    messages.iter().any(|message| {
        let Json::Object(fields) = message else {
            return false;
        };
        let Some(Json::List(blocks)) = fields.get("content") else {
            return false;
        };
        blocks.iter().any(|block| {
            let Json::Object(fields) = block else {
                return false;
            };
            matches!(fields.get("type"), Some(Json::Text(kind)) if kind.as_str() == COMPACTION_BLOCK_TYPE)
        })
    })
}

// the three request-wide values the handler passes on: the block verdict, the proxy metadata and the router
pub struct ContextInspection<'router> {
    pub has_compaction_block: bool,
    pub proxy: Option<ProxyContext>,
    pub router: Option<&'router dyn SummaryCompletion>,
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn inspect_compaction_and_proxy_context<'router>(
    messages: &[Json],
    kwargs: &Extra,
    router: Option<&'router dyn SummaryCompletion>,
) -> ContextInspection<'router> {
    ContextInspection {
        has_compaction_block: has_compaction_block(messages),
        proxy: extract_proxy_metadata(kwargs),
        router,
    }
}
