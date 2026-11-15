use crate::huggingface_chat::HuggingfaceEnv;

const DEFAULT_API_BASE: &str = "https://api-inference.huggingface.co";
const RERANK_PATH: &str = "/rerank";

#[must_use]
pub fn get_api_base(api_base: Option<&str>, env: &HuggingfaceEnv) -> String {
    if let Some(base) = api_base {
        return base.to_owned();
    }
    if let Some(base) = env.hf_api_base.as_deref() {
        return base.to_owned();
    }
    if let Some(base) = env.huggingface_api_base.as_deref() {
        return base.to_owned();
    }
    DEFAULT_API_BASE.to_owned()
}

#[must_use]
pub fn get_complete_url(api_base: Option<&str>, env: &HuggingfaceEnv) -> String {
    let trimmed = get_api_base(api_base, env);
    let mut base = trimmed.trim_end_matches('/').to_owned();
    if !base.ends_with(RERANK_PATH) {
        base.push_str(RERANK_PATH);
    }
    base
}
