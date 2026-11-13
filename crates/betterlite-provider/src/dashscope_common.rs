use url::{Position, Url};

pub const DASHSCOPE_CHAT_COMPATIBLE_PATH: &str = "/compatible-mode/v1";
pub const DASHSCOPE_RERANK_PATH: &str = "/compatible-api/v1/reranks";
pub const DASHSCOPE_RERANK_URL: &str = "https://dashscope.aliyuncs.com/compatible-api/v1/reranks";

mod tests;

#[must_use]
pub fn resolve_rerank_base(
    api_base: Option<&str>,
    env_value: &str,
    default_rerank_base: &str,
) -> String {
    let remapped = rerank_base_for_chat_shaped_base(api_base);
    match (api_base, remapped) {
        // a base that is not chat-shaped is the caller's own rerank route, even when it is empty
        (Some(base), None) => base.to_owned(),
        (_, Some(route)) if env_value.is_empty() => route,
        _ if !env_value.is_empty() => env_value.to_owned(),
        _ => default_rerank_base.to_owned(),
    }
}

fn rerank_base_for_chat_shaped_base(api_base: Option<&str>) -> Option<String> {
    let api_base = api_base?;
    let parsed = Url::parse(api_base).ok()?;
    let host = parsed.host_str()?.to_lowercase();
    let on_aliyun_host = host == "aliyuncs.com" || host.ends_with(".aliyuncs.com");
    if !on_aliyun_host || parsed.path().trim_end_matches('/') != DASHSCOPE_CHAT_COMPATIBLE_PATH {
        return None;
    }
    let scheme = parsed.scheme();
    let netloc = &parsed[Position::BeforeHost..Position::AfterPort];
    Some(format!("{scheme}://{netloc}{DASHSCOPE_RERANK_PATH}"))
}
