use betterlite_guardrails::guardrail_message_scope::Object;
use indexmap::IndexMap;

use self::streaming::apply_output_streaming;
use super::error::RewriteError;
use super::response::ChatResponse;
use super::string_choice_contents::string_choice_contents;

pub mod streaming;

mod tests;

// One rewrite per rebuilt choice index, kept only when the guardrail changed a string content; a later choice with the same index wins.
#[must_use]
pub fn text_rewrites_by_choice(
    guardrailed: &ChatResponse,
    pre_guardrail_texts: &[Option<String>],
) -> IndexMap<i64, String> {
    let post_guardrail_texts = string_choice_contents(guardrailed);
    let mut rewrites = IndexMap::new();
    for ((choice, before), after) in guardrailed
        .choices
        .iter()
        .zip(pre_guardrail_texts)
        .zip(post_guardrail_texts)
    {
        if let (Some(old), Some(new)) = (before, after)
            && *old != new
        {
            rewrites.insert(choice.index, new);
        }
    }
    rewrites
}

// The full rewritten text goes to a choice's first content-carrying chunk and the later ones are blanked.
pub fn write_ended_text_rewrites(
    responses_so_far: &mut [Object],
    guardrailed: &ChatResponse,
    pre_guardrail_texts: &[Option<String>],
) -> Result<(), RewriteError> {
    let rewrites = text_rewrites_by_choice(guardrailed, pre_guardrail_texts);
    if rewrites.is_empty() {
        return Ok(());
    }
    let mappings: Vec<_> = rewrites.keys().map(|&index| (index, None)).collect();
    let texts: Vec<String> = rewrites.into_values().collect();
    apply_output_streaming(responses_so_far, &texts, &mappings)
}
