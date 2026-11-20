use betterlite_models::model_capability_predicates::CapabilityPredicates;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::api::messages_chat_content::{ChatTarget, translate_messages};
use crate::api::messages_chat_options::context::OptionsContext;
use crate::api::messages_chat_options::metadata::translate_metadata;
use crate::api::messages_chat_options::passthrough::{
    copy_untranslated_params, translate_stop_sequences,
};
use crate::api::messages_chat_options::system::insert_system_message;
use crate::api::messages_chat_options::thinking::translate_thinking;
use crate::api::messages_chat_tools_schema::request::{
    translate_output_format, translate_tool_choice, translate_tools,
};
use crate::api::messages_chat_tools_schema::tools::ToolNameMapping;

use super::error::TransformError;
use super::settings::AdapterSettings;

// LiteLLM's translate_anthropic_to_openai: every step in its declared order, the copy-through last.
pub async fn translate_anthropic_to_openai<Predicates>(
    request: &mut Object,
    settings: &AdapterSettings<'_>,
    predicates: &mut Predicates,
) -> Result<(Object, ToolNameMapping), TransformError>
where
    Predicates: CapabilityPredicates + Send,
{
    let (messages, model) = conversation(request, settings, predicates).await?;
    let mut kwargs = Object::new();
    kwargs.insert("model".to_owned(), Json::Text(model.clone()));
    kwargs.insert("messages".to_owned(), Json::List(messages));
    let options = OptionsContext {
        model: model.as_str(),
        custom_llm_provider: settings.custom_llm_provider,
        auto_summary: settings.auto_summary,
        thresholds: settings.thresholds,
        limits: settings.limits,
        parameters: settings.parameters,
    };
    let tool_name_mapping = translate_options(request, &mut kwargs, &options)?;
    Ok((kwargs, tool_name_mapping))
}

// LiteLLM reads messages first, translates them, and only then needs the model new_kwargs carries.
async fn conversation<Predicates>(
    request: &Object,
    settings: &AdapterSettings<'_>,
    predicates: &mut Predicates,
) -> Result<(Vec<Json>, String), TransformError>
where
    Predicates: CapabilityPredicates + Send,
{
    let Some(sent_messages) = request.get("messages") else {
        return Err(TransformError::MissingKey { key: "messages" });
    };
    let Json::List(messages) = sent_messages else {
        return Err(TransformError::MessagesNotList);
    };
    let model = match request.get("model") {
        None => None,
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(TransformError::ModelNotText);
        }
    };
    let target = ChatTarget {
        model,
        custom_llm_provider: settings.custom_llm_provider,
    };
    let mut translated = translate_messages(
        messages,
        &target,
        settings.preserve_midturn_system,
        predicates,
    )
    .await?;
    insert_system_message(&mut translated, request);
    let Some(model_name) = model else {
        return Err(TransformError::MissingKey { key: "model" });
    };
    Ok((translated, model_name.to_owned()))
}

// The option steps in order: the copy-through is last because it may overwrite anything translated.
fn translate_options(
    request: &mut Object,
    kwargs: &mut Object,
    options: &OptionsContext<'_>,
) -> Result<ToolNameMapping, TransformError> {
    translate_metadata(request, kwargs, options)?;
    translate_tool_choice(request, kwargs)?;
    let tool_name_mapping = translate_tools(request, kwargs)?;
    translate_thinking(request, kwargs, options)?;
    translate_stop_sequences(request, kwargs);
    translate_output_format(request, kwargs)?;
    copy_untranslated_params(request, kwargs);
    Ok(tool_name_mapping)
}
