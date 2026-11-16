use betterlite_context::chat_config_selection::tags::ChatConfigTag;

use super::a2a::complete_a2a;
use super::context::DispatchContext;
use super::error::DispatchError;
use super::handler::ChatCompletionHandler;
use super::langflow::complete_langflow;
use super::langgraph::complete_langgraph;

// LiteLLM's dispatch arms for the three agent platforms; every other provider keeps its own path
#[must_use]
pub fn dispatch<R>(
    tag: ChatConfigTag,
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Option<Result<R, DispatchError>> {
    if tag == ChatConfigTag::A2a {
        return Some(complete_a2a(context, handler));
    }
    if tag == ChatConfigTag::LangGraph {
        return Some(complete_langgraph(context, handler));
    }
    if tag == ChatConfigTag::LangFlow {
        return Some(complete_langflow(context, handler));
    }
    None
}
