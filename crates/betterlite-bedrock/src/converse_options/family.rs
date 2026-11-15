use unicode_general_category::{GeneralCategory, get_general_category};

pub const DEEPSEEK_MARKER: &str = "deepseek";
pub const DEEPSEEK_R1_MARKER: &str = "deepseek.r1";

const CONVERSE_WRAPPERS: [&str; 3] = ["bedrock/converse/", "bedrock/", "converse/"];
const NOVA_2_PREFIXES: [&str; 2] = ["amazon.nova-2-", "nova-2/"];
const OPENAI_GPT_MARKER: &str = "openai.gpt-";
const REGION_PREFIXES: [&str; 3] = ["us.", "eu.", "apac."];
const XAI_GROK_MARKER: &str = "xai.grok-";

#[must_use]
pub fn is_openai_gpt_reasoning_model(model: &str) -> bool {
    for (start, _) in model.match_indices(OPENAI_GPT_MARKER) {
        let Some(tail) = model
            .get(start..)
            .and_then(|text| text.strip_prefix(OPENAI_GPT_MARKER))
        else {
            continue;
        };
        if tail
            .chars()
            .next()
            .is_some_and(|next| get_general_category(next) == GeneralCategory::DecimalNumber)
        {
            return true;
        }
    }
    false
}

#[must_use]
pub fn requires_min_max_tokens(model: &str) -> bool {
    if model.contains(XAI_GROK_MARKER) {
        return true;
    }
    is_openai_gpt_reasoning_model(model)
}

#[must_use]
pub fn is_nova_2_model(model: &str) -> bool {
    let without_wrapper = strip_first_prefix(model, &CONVERSE_WRAPPERS);
    let without_region = strip_first_prefix(without_wrapper, &REGION_PREFIXES);
    NOVA_2_PREFIXES
        .iter()
        .any(|prefix| without_region.starts_with(prefix))
}

#[must_use]
pub fn is_deepseek_model(model: &str, base_model: &str) -> bool {
    [model, base_model]
        .into_iter()
        .any(|name| name.contains(DEEPSEEK_MARKER))
}

// DeepSeek R1 always reasons, so it rejects every reasoning request shape the other DeepSeek models accept.
#[must_use]
pub fn rejects_reasoning_effort(model: &str, base_model: &str) -> bool {
    [model, base_model]
        .into_iter()
        .any(|name| name.contains(DEEPSEEK_R1_MARKER))
}

fn strip_first_prefix<'model>(model: &'model str, prefixes: &[&str]) -> &'model str {
    prefixes
        .iter()
        .copied()
        .find_map(|prefix| model.strip_prefix(prefix))
        .unwrap_or(model)
}
