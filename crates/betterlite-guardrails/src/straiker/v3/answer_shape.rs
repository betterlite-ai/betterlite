use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::response_tool_calls::structured::RouteCallTypes;

use crate::straiker::v3::completion_as_chat::{TextCompletionResponse, text_completion_as_chat};
use crate::straiker::v3::route_detection::route_is;
use crate::straiker_webhook::jsonable::jsonable_dict;

// The proxy's Messages adapter, owned by the message output adapters: it turns a chat completion back into the Anthropic Messages answer.
pub trait AnthropicTranslator {
    fn translate(&self, response: &Object) -> Object;
}

// What the proxy holds as the answer. `Chat` is a chat completion dumped with unset fields left out.
pub enum Response {
    TextCompletion(TextCompletionResponse),
    Chat(Object),
    Other(Json),
}

pub struct AnswerContext<'context> {
    pub request_data: &'context Object,
    pub routes: &'context dyn RouteCallTypes<CallType>,
    pub translator: &'context dyn AnthropicTranslator,
    pub model: Option<&'context str>,
}

// The answer in the API shape the client spoke. On a streamed Messages call the proxy rebuilds the answer as a chat completion before the hook runs, and Straiker's coding-agent reader parses a Messages answer, so the adapter turns it back.
#[must_use]
pub fn answer(response: &Response, context: &AnswerContext<'_>) -> Option<Object> {
    match response {
        Response::TextCompletion(completion) => Some(text_completion_as_chat(completion)),
        Response::Chat(chat) => {
            let messages_route = route_is(
                context.request_data,
                CallType::AnthropicMessages,
                context.routes,
            );
            if !messages_route {
                return Some(chat.clone());
            }
            let mut translated = context.translator.translate(chat);
            let model = chat
                .get("model")
                .and_then(Json::as_str)
                .filter(|text| !text.is_empty())
                .or(context.model);
            translated.insert(
                "model".to_owned(),
                model.map_or(Json::Null, |name| Json::Text(name.to_owned())),
            );
            Some(translated)
        }
        Response::Other(value) => jsonable_dict(value),
    }
}

mod tests;
