use betterlite_prompts::arguments::json::Json;

use super::FeatureParams;

#[must_use]
pub fn requires_compaction_beta(
    provider: &str,
    params: Option<&FeatureParams>,
    messages: &[Json],
) -> bool {
    if provider != "anthropic" {
        return false;
    }
    if params
        .and_then(|source| source.compaction.as_ref())
        .is_some()
    {
        return true;
    }
    messages.iter().any(|message| {
        let Some(fields) = message.as_object() else {
            return false;
        };
        let Some(Json::List(blocks)) = fields.get("content") else {
            return false;
        };
        blocks.iter().any(is_signed_compaction_block)
    })
}

fn is_signed_compaction_block(block: &Json) -> bool {
    let Some(fields) = block.as_object() else {
        return false;
    };
    if fields.get("type").and_then(Json::as_str) != Some("compaction") {
        return false;
    }
    matches!(
        fields.get("signature").and_then(Json::as_str),
        Some(signature) if !signature.is_empty()
    )
}
