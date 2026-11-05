use betterlite_core::agents::json::AgentJson;
use serde::Deserialize;
use serde_with::{DefaultOnError, DefaultOnNull, serde_as};

mod tests;

#[serde_as]
#[derive(Debug, Default, Deserialize)]
pub struct WxoSendParams {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub message: WxoSendMessage,
}

#[serde_as]
#[derive(Debug, Default, Deserialize)]
pub struct WxoSendMessage {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub parts: Vec<AgentJson>,
}

// WXO answers with several shapes, so every level falls back to empty instead of failing
#[serde_as]
#[derive(Debug, Default, Deserialize)]
pub struct WxoRunResult {
    #[serde(default)]
    #[serde_as(as = "DefaultOnError")]
    last_message: Option<WxoMessageBody>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnError")]
    result: Option<WxoNestedResult>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnError")]
    results: Option<String>,
}

#[serde_as]
#[derive(Debug, Default, Deserialize)]
struct WxoNestedResult {
    #[serde(default)]
    #[serde_as(as = "DefaultOnError")]
    data: Option<WxoNestedData>,
}

#[serde_as]
#[derive(Debug, Default, Deserialize)]
struct WxoNestedData {
    #[serde(default)]
    #[serde_as(as = "DefaultOnError")]
    message: Option<WxoMessageBody>,
}

#[serde_as]
#[derive(Debug, Default, Deserialize)]
struct WxoMessageBody {
    #[serde(default)]
    #[serde_as(as = "DefaultOnError<Vec<DefaultOnError>>")]
    content: Vec<WxoContentItem>,
}

#[serde_as]
#[derive(Debug, Default, Deserialize)]
struct WxoContentItem {
    #[serde(default)]
    #[serde_as(as = "DefaultOnError")]
    text: Option<String>,
}

#[must_use]
pub fn extract_text_from_a2a_params(params: &WxoSendParams) -> String {
    let texts = params.message.parts.iter().filter_map(|item| {
        let part = item.as_object()?;
        let kind = part.get("kind").unwrap_or(&AgentJson::Null);
        if !kind.is_null() && !matches!(kind.as_str(), Some("" | "text")) {
            return None;
        }
        part.get("text")?.as_str().filter(|text| !text.is_empty())
    });
    texts.collect::<Vec<_>>().join(" ")
}

#[must_use]
pub fn extract_text_from_wxo_result(run: &WxoRunResult) -> String {
    let nested = run
        .result
        .as_ref()
        .and_then(|result| result.data.as_ref())
        .and_then(|data| data.message.as_ref());
    // an empty text counts as missing, so the next location is tried
    let text = [run.last_message.as_ref(), nested]
        .into_iter()
        .flatten()
        .filter_map(|body| body.content.first()?.text.as_deref())
        .chain(run.results.as_deref())
        .find(|text| !text.is_empty());
    text.unwrap_or_default().to_owned()
}
