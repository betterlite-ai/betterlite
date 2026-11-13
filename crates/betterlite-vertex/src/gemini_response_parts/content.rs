use crate::gemini_response_parts::media::audio_uri;
use crate::gemini_response_parts::projection::Part;
use serde::Serialize;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThinkingBlock {
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
}
pub(crate) fn assistant_content(parts: &[Part]) -> (Option<String>, Option<String>) {
    let mut content = String::new();
    let mut reasoning = String::new();
    for part in parts {
        let piece = match (&part.text, &part.inline_data) {
            (Some(text), _) => audio_uri(text).is_none().then(|| text.clone()),
            (None, Some(inline)) => (!inline.mime_type.starts_with("audio/")
                && !inline.mime_type.starts_with("image/"))
            .then(|| format!("data:{};base64,{}", inline.mime_type, inline.data)),
            (None, None) => None,
        };
        let Some(text) = piece.filter(|text| !text.is_empty()) else {
            continue;
        };
        if part.thought {
            reasoning.push_str(&text);
        } else {
            content.push_str(&text);
        }
    }
    (
        (!content.is_empty()).then_some(content),
        (!reasoning.is_empty()).then_some(reasoning),
    )
}
pub(crate) fn thinking_blocks(parts: &[Part]) -> Vec<ThinkingBlock> {
    parts
        .iter()
        .filter(|part| part.thought)
        .map(|part| ThinkingBlock::Thinking {
            thinking: part.text.clone().unwrap_or_default(),
            signature: part.thought_signature.clone(),
        })
        .collect()
}
pub(crate) fn thought_signatures(parts: &[Part]) -> Option<Vec<String>> {
    let signatures: Vec<String> = parts
        .iter()
        .filter_map(|part| part.thought_signature.clone())
        .collect();
    (!signatures.is_empty()).then_some(signatures)
}
