#![cfg(test)]

use super::*;

fn assistant_message(content: &str, provider_specific_fields: Object) -> Message {
    Message {
        role: "assistant".to_owned(),
        content: Some(Content::Text(content.to_owned())),
        name: None,
        provider_specific_fields,
    }
}

mod ignores_non_true_prefix_values;
mod rewrites_a_true_prefix_message;
