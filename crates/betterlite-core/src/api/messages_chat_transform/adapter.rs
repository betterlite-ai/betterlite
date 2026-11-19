use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_models::model_capability_predicates::CapabilityPredicates;
use betterlite_prompts::arguments::json::Object;

use crate::api::messages_chat_output::error::ChatOutputError;
use crate::api::messages_chat_output::output::MessageResponse;
use crate::api::messages_chat_output::records::ResponseRecord;
use crate::api::messages_chat_output::response::response_message;
use crate::api::messages_chat_tools_schema::tools::ToolNameMapping;
use crate::api::messages_context_clear::result::PolyfillResult;
use crate::api::messages_stream_split::{AsyncChunkSource, ChunkSource};

use super::compose::translate_anthropic_to_openai;
use super::error::TransformError;
use super::settings::AdapterSettings;
use super::stream::{StreamOptions, StreamOutput, translate_output_stream};

// LiteLLM's AnthropicAdapter holds no state: every call carries everything it needs.
pub struct AnthropicAdapter;

impl Default for AnthropicAdapter {
    fn default() -> Self {
        Self
    }
}

impl AnthropicAdapter {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub const fn new() -> Self {
        Self
    }

    // ast-grep-ignore: no-long-names
    pub async fn translate_completion_input_params_with_tool_mapping<Predicates>(
        &self,
        kwargs: &mut Object,
        settings: &AdapterSettings<'_>,
        predicates: &mut Predicates,
    ) -> Result<(Object, ToolNameMapping), TransformError>
    where
        Predicates: CapabilityPredicates + Send,
    {
        // Both required keys leave kwargs before either is checked, so a missing messages key still consumes model.
        let mut request = request_body(kwargs)?;
        translate_anthropic_to_openai(&mut request, settings, predicates).await
    }

    // ast-grep-ignore: no-long-names
    pub async fn translate_completion_input_params<Predicates: CapabilityPredicates + Send>(
        &self,
        kwargs: &mut Object,
        settings: &AdapterSettings<'_>,
        predicates: &mut Predicates,
    ) -> Result<Object, TransformError> {
        // LiteLLM's convenience method takes no provider keyword, so it composes as if none was named.
        let mut unrouted = *settings;
        unrouted.custom_llm_provider = None;
        let (request, _tool_name_mapping) = self
            .translate_completion_input_params_with_tool_mapping(kwargs, &unrouted, predicates)
            .await?;
        Ok(request)
    }

    // ast-grep-ignore: no-long-names, no-one-line-functions
    pub fn translate_completion_output_params(
        &self,
        response: &ResponseRecord,
        tool_name_mapping: Option<&ToolNameMapping>,
        polyfill_result: Option<&PolyfillResult>,
    ) -> Result<MessageResponse, ChatOutputError> {
        response_message(response, tool_name_mapping, polyfill_result)
    }

    // is_async keeps LiteLLM's default: an async caller that forgot the flag still gets its async iterator.
    #[must_use]
    // ast-grep-ignore: no-long-names, no-one-line-functions
    pub fn translate_completion_output_params_streaming<S, I>(
        &self,
        source: S,
        options: StreamOptions<'_>,
        ids: I,
    ) -> StreamOutput<S, I>
    where
        S: AsyncChunkSource + ChunkSource + Send,
        I: IdSource,
    {
        translate_output_stream(source, options, ids)
    }
}

// LiteLLM's Typed Request Body: model and messages come first, every other kwarg follows as it was sent.
fn request_body(kwargs: &mut Object) -> Result<Object, TransformError> {
    let Some(model) = kwargs.shift_remove("model") else {
        return Err(TransformError::MissingKey { key: "model" });
    };
    let Some(messages) = kwargs.shift_remove("messages") else {
        return Err(TransformError::MissingKey { key: "messages" });
    };
    if !model.is_truthy() {
        return Err(TransformError::ModelRequired);
    }
    if !messages.is_truthy() {
        return Err(TransformError::MessagesRequired);
    }
    let mut request = Object::new();
    request.insert("model".to_owned(), model);
    request.insert("messages".to_owned(), messages);
    for (key, value) in kwargs.iter() {
        request.insert(key.clone(), value.clone());
    }
    Ok(request)
}
