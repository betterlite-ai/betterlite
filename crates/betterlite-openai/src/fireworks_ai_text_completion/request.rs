use betterlite_models::fireworks_catalog_metadata::resolve_resource_name;
use serde_json::Value;

use super::FireworksCompletionError;
use super::extra_body::map_extra_body_params;
use crate::fireworks_ai_chat_params::FireworksCapabilities;
use crate::openai_base_config::Object;
use crate::text_completion_policy::prompt;

// LiteLLM's transform_text_completion_request: translate the options, project the prompt, then spread both into the body.
pub async fn transform_completion_request<P>(
    capabilities: &mut P,
    model: &str,
    messages: &[Value],
    optional_params: &Object,
) -> Result<Object, FireworksCompletionError>
where
    P: FireworksCapabilities,
{
    let translated = map_extra_body_params(capabilities, model, optional_params).await;
    let projected = prompt::transform_prompt(messages)?;
    let mut data = Object::new();
    data.insert(
        String::from("model"),
        Value::String(resolve_resource_name(model)),
    );
    data.insert(String::from("prompt"), projected);
    data.extend(translated);
    Ok(data)
}
