use crate::huggingface_chat::HuggingfaceEnv;

const ROUTER: &str = "https://router.huggingface.co/hf-inference/pipeline";

#[must_use]
pub fn embedding_url(
    model: &str,
    api_base: Option<&str>,
    env: &HuggingfaceEnv,
    task: Option<&str>,
) -> String {
    if model.starts_with("http://") || model.starts_with("https://") {
        return model.to_owned();
    }
    if let Some(base) = api_base.filter(|base| !base.is_empty()) {
        return base.to_owned();
    }
    // a base present in the environment counts even when its value is empty
    if let Some(base) = env.hf_api_base.as_deref() {
        return base.to_owned();
    }
    if let Some(base) = env.huggingface_api_base.as_deref() {
        return base.to_owned();
    }
    let resolved_task = task.unwrap_or("None");
    format!("{ROUTER}/{resolved_task}/{model}")
}
