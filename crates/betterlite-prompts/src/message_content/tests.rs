#![cfg(test)]

use super::*;

fn message(role: &str, content: Option<Content>) -> Message {
    Message {
        role: role.to_owned(),
        content,
        name: None,
        provider_specific_fields: Object::new(),
    }
}

mod content_conversion;
mod content_list_in_place_conversion;
mod non_content_values_ignore_name;
mod role_and_content_projection;
mod search_results_and_null_removal;
