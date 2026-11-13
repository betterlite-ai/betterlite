use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, Message};

const CACHE_CONTROL: &str = "cache_control";
// LiteLLM stops the recursive cache_control walk at its shared DEFAULT_MAX_RECURSE_DEPTH
const MAX_DEPTH: usize = 100;

pub(super) fn strip_message(message: &mut Message) {
    strip_object(&mut message.provider_specific_fields, 0);
    // LiteLLM counts the message at depth 0 and each content part, an item of its content list, at depth 1
    if let Some(Content::Parts(parts)) = &mut message.content {
        for part in parts {
            strip_object(&mut part.fields, 1);
        }
    }
}

pub(super) fn strip_tools(optional_params: &mut Object) {
    let Some(Json::List(tools)) = optional_params.get("tools") else {
        return;
    };
    let mut filtered = tools.clone();
    for tool in &mut filtered {
        if let Json::Object(object) = tool {
            strip_object(object, 0);
        }
    }
    optional_params.insert("tools".to_owned(), Json::List(filtered));
}

fn strip_object(object: &mut Object, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    object.shift_remove(CACHE_CONTROL);
    for value in object.values_mut() {
        strip_value(value, depth.saturating_add(1));
    }
}

fn strip_value(value: &mut Json, depth: usize) {
    match value {
        Json::Object(object) => strip_object(object, depth),
        // LiteLLM recurses a dict inside a list at the list owner's depth plus one, with no extra step for the list
        Json::List(items) => {
            for item in items {
                if let Json::Object(object) = item {
                    strip_object(object, depth);
                }
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
}
