#![cfg(test)]

use super::*;
use crate::arguments::json::{Json, Object};

fn message(provider_specific_fields: Object) -> Message {
    Message {
        role: "assistant".to_owned(),
        content: None,
        name: None,
        provider_specific_fields,
    }
}

mod strip_internal_fields;
