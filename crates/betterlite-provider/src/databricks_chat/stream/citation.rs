use betterlite_prompts::arguments::json::{Json, Object};

use crate::databricks_chat::error::DatabricksChatError;

pub(super) fn add(delta: &mut Object) -> Result<(), DatabricksChatError> {
    let Some(citation) = citation_of(delta)? else {
        return Ok(());
    };
    match delta.get_mut("provider_specific_fields") {
        Some(Json::Object(fields)) => {
            fields.insert(String::from("citation"), citation);
        }
        Some(_) => {
            return Err(DatabricksChatError::wrong_type(
                "provider_specific_fields of a Databricks stream delta",
                "an object",
            ));
        }
        None => {
            let mut fields = Object::new();
            fields.insert(String::from("citation"), citation);
            delta.insert(
                String::from("provider_specific_fields"),
                Json::Object(fields),
            );
        }
    }
    Ok(())
}

// The first citation of the first content block, when there is one
fn citation_of(delta: &Object) -> Result<Option<Json>, DatabricksChatError> {
    let Some(Json::List(items)) = delta.get("content") else {
        return Ok(None);
    };
    let Some(first) = items.first() else {
        return Ok(None);
    };
    let Some(first) = first.as_object() else {
        return Err(DatabricksChatError::wrong_type(
            "a content block of a Databricks stream delta",
            "an object",
        ));
    };
    let Some(citations) = first.get("citations") else {
        return Ok(None);
    };
    if !citations.is_truthy() {
        return Ok(None);
    }
    let Json::List(entries) = citations else {
        return Err(DatabricksChatError::wrong_type(
            "citations of a content block",
            "a list",
        ));
    };
    Ok(entries.first().cloned())
}
