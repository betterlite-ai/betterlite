use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// StandardLoggingPayloadSetup.append_system_prompt_messages: a system prompt already at the head is kept, otherwise it is prepended.
#[must_use]
pub fn append_system_prompt_messages(
    system: Option<&str>,
    messages: Option<&Json>,
) -> Option<Json> {
    let Some(prompt) = system else {
        return messages.cloned();
    };
    match messages {
        None => Some(prepend_system(prompt, Vec::new())),
        Some(Json::Text(text)) => {
            let user = message("user", text);
            Some(prepend_system(prompt, vec![user]))
        }
        Some(Json::List(items)) => {
            let duplicate = matches!(
                items.first(),
                Some(Json::Object(first))
                    if matches!(first.get("role"), Some(Json::Text(role)) if role == "system")
                        && matches!(first.get("content"), Some(Json::Text(content)) if content == prompt)
            );
            if duplicate {
                return Some(Json::List(items.clone()));
            }
            Some(prepend_system(prompt, items.clone()))
        }
        Some(other) => Some(other.clone()),
    }
}

// A message map in the shape the declared messages type carries.
fn message(role: &str, content: &str) -> Json {
    Json::Object(IndexMap::from([
        (String::from("role"), Json::Text(role.to_owned())),
        (String::from("content"), Json::Text(content.to_owned())),
    ]))
}

fn prepend_system(system: &str, mut rest: Vec<Json>) -> Json {
    rest.insert(0, message("system", system));
    Json::List(rest)
}

mod tests;
