use crate::huggingface_chat::HuggingfaceEnv;
use crate::huggingface_chat::error::HuggingfaceChatError;

const BASE_URL: &str = "https://router.huggingface.co";
const CHAT_COMPLETIONS: &str = "/chat/completions";
const V1_CHAT_COMPLETIONS: &str = "/v1/chat/completions";

#[must_use]
pub fn build_chat_completion_url(model_url: &str) -> String {
    let mut url = model_url.trim_end_matches('/').to_owned();
    if url.ends_with("/v1") {
        url.push_str(CHAT_COMPLETIONS);
    }
    if !url.ends_with(CHAT_COMPLETIONS) {
        url.push_str(V1_CHAT_COMPLETIONS);
    }
    url
}

pub fn get_complete_url(
    api_base: Option<&str>,
    model: &str,
    env: &HuggingfaceEnv,
) -> Result<String, HuggingfaceChatError> {
    let complete_url = if let Some(base) = api_base {
        build_chat_completion_url(base)
    } else if let Some(base) = env_base(env) {
        base
    } else if model.starts_with("http://") || model.starts_with("https://") {
        build_chat_completion_url(model)
    } else {
        router_url(model)?
    };
    Ok(complete_url.trim_end_matches('/').to_owned())
}

// LiteLLM's str(os.getenv("HF_API_BASE")) turns an unset variable into the text "None"
fn env_base(env: &HuggingfaceEnv) -> Option<String> {
    if let Some(hf_api_base) = env.hf_api_base.as_deref().filter(|base| !base.is_empty()) {
        return Some(hf_api_base.to_owned());
    }
    let huggingface_api_base = env
        .huggingface_api_base
        .as_deref()
        .filter(|base| !base.is_empty())?;
    if env.hf_api_base.is_none() {
        return Some("None".to_owned());
    }
    Some(huggingface_api_base.to_owned())
}

fn router_url(model: &str) -> Result<String, HuggingfaceChatError> {
    let (first_part, remaining) =
        model
            .split_once('/')
            .ok_or_else(|| HuggingfaceChatError::MissingProviderPrefix {
                model: model.to_owned(),
            })?;
    if !remaining.contains('/') {
        return Ok(format!("{BASE_URL}{V1_CHAT_COMPLETIONS}"));
    }
    let route = match first_part {
        "hf-inference" => format!("{first_part}/models/{model}/v1/chat/completions"),
        "novita" => format!("{first_part}/v3/openai/chat/completions"),
        "fireworks-ai" => format!("{first_part}/inference/v1/chat/completions"),
        provider => format!("{provider}/v1/chat/completions"),
    };
    Ok(format!("{BASE_URL}/{route}"))
}
