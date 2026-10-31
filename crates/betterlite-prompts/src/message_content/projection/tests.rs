#![cfg(test)]

use super::*;
use crate::arguments::json::Object;
use crate::message_content::Content;

fn message(role: &str, content: &str) -> Message {
    Message {
        role: role.to_owned(),
        content: Some(Content::Text(content.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

mod role_and_content_projection;
mod role_projection_dropped;
mod role_projection_kept;
