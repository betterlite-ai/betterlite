use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

use crate::guardrails::chat_guardrail_translation::extraction::apply_error::ApplyError;
use crate::guardrails::chat_guardrail_translation::extraction::combine_texts::TextKey;
use crate::guardrails::chat_guardrail_translation::extraction::json_access::{
    field, field_mut, integer_value,
};
use crate::guardrails::chat_guardrail_translation::rewrites::error::RewriteError;

mod tests;

// The choice's message when it has one, else its delta.
fn payload_mut(choice: &mut GuardrailValue) -> Option<&mut GuardrailValue> {
    if field(choice, "message").is_some() {
        field_mut(choice, "message")
    } else {
        field_mut(choice, "delta")
    }
}

struct Rewrites<'texts> {
    by_key: BTreeMap<TextKey, &'texts str>,
    written: BTreeSet<TextKey>,
}

impl Rewrites<'_> {
    // The first slot of a key takes the rewrite and every later one is blanked.
    fn text_for(&mut self, key: TextKey) -> Option<String> {
        let rewrite = self.by_key.get(&key)?;
        Some(if self.written.insert(key) {
            (*rewrite).to_owned()
        } else {
            String::new()
        })
    }
}

fn rewrite_choice(choice: &mut GuardrailValue, rewrites: &mut Rewrites<'_>) {
    let index = integer_value(field(choice, "index")).unwrap_or(0);
    let Some(content) = payload_mut(choice).and_then(|payload| field_mut(payload, "content"))
    else {
        return;
    };
    match content {
        GuardrailValue::Text(current) => {
            if let Some(text) = rewrites.text_for((index, None)) {
                *current = text;
            }
        }
        GuardrailValue::Array(items) => {
            for (content_index, item) in items.iter_mut().enumerate() {
                let GuardrailValue::Object(fields) = item else {
                    continue;
                };
                if !fields.contains_key("text") {
                    continue;
                }
                if let Some(text) = rewrites.text_for((index, Some(content_index))) {
                    fields.insert("text".to_owned(), GuardrailValue::Text(text));
                }
            }
        }
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Object(_) => {}
    }
}

// `mappings` keys are the choice's own index, not its position in a chunk's list.
pub fn apply_output_streaming(
    responses: &mut [Object],
    guardrailed_texts: &[String],
    mappings: &[TextKey],
) -> Result<(), RewriteError> {
    let mut rewrites = Rewrites {
        by_key: BTreeMap::new(),
        written: BTreeSet::new(),
    };
    for (response_index, text) in guardrailed_texts.iter().enumerate() {
        let Some(&key) = mappings.get(response_index) else {
            return Err(RewriteError::from(ApplyError::UnmappedResponse {
                response_index,
            }));
        };
        rewrites.by_key.insert(key, text.as_str());
    }
    for response in responses {
        let Some(GuardrailValue::Array(choices)) = response.get_mut("choices") else {
            continue;
        };
        for choice in choices {
            rewrite_choice(choice, &mut rewrites);
        }
    }
    Ok(())
}
